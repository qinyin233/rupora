use std::{
    collections::HashSet,
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

const PROTOCOL_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
const MAX_SERVICES: usize = 32;
const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const HARD_MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_OUTPUT_BYTES: usize = 1024 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 5_000;
const HARD_MAX_TIMEOUT_MS: u64 = 30_000;
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionPermission {
    ReadDocument,
    ReadDocumentPath,
    ReplaceDocument,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionService {
    pub name: String,
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub permissions: HashSet<ExtensionPermission>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_output_bytes")]
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    services: Vec<ExtensionService>,
}

#[derive(Clone, Debug)]
pub struct ExtensionRegistry {
    path: PathBuf,
    config: ExtensionConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionInvocation {
    pub replacement: Option<String>,
    pub message: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionRequest<'a> {
    protocol: u32,
    request_id: u64,
    method: &'static str,
    document: ExtensionDocument<'a>,
}

#[derive(Serialize)]
struct ExtensionDocument<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<&'a Path>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExtensionResponse {
    protocol: u32,
    request_id: u64,
    #[serde(default)]
    result: Option<ExtensionResult>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionResult {
    #[serde(default)]
    replacement: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

impl ExtensionRegistry {
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let config = if path.exists() {
            let metadata = fs::metadata(&path)
                .map_err(|error| format!("无法检查扩展配置 {}：{error}", path.display()))?;
            if metadata.len() > MAX_CONFIG_BYTES {
                return Err("扩展配置超过 256 KiB 上限".to_owned());
            }
            let bytes = fs::read(&path)
                .map_err(|error| format!("无法读取扩展配置 {}：{error}", path.display()))?;
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("扩展配置无效 {}：{error}", path.display()))?
        } else {
            ExtensionConfig::default()
        };
        validate_config(&config)?;
        Ok(Self { path, config })
    }

    pub fn disabled(path: PathBuf) -> Self {
        Self {
            path,
            config: ExtensionConfig::default(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn services(&self) -> &[ExtensionService] {
        if self.config.enabled {
            &self.config.services
        } else {
            &[]
        }
    }

    pub fn config_path(&self) -> &Path {
        &self.path
    }

    pub fn reload(&mut self) -> Result<(), String> {
        *self = Self::load(self.path.clone())?;
        Ok(())
    }

    pub fn ensure_template(&self) -> Result<(), String> {
        if self.path.exists() {
            return Ok(());
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "扩展配置没有父目录".to_owned())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建扩展配置目录 {}：{error}", parent.display()))?;
        let bytes = serde_json::to_vec_pretty(&ExtensionConfig::default())
            .map_err(|error| format!("无法生成扩展配置：{error}"))?;
        fs::write(&self.path, bytes)
            .map_err(|error| format!("无法写入扩展配置 {}：{error}", self.path.display()))
    }
}

pub fn invoke(
    service: &ExtensionService,
    document: &str,
    document_path: Option<&Path>,
) -> Result<ExtensionInvocation, String> {
    validate_service(service)?;
    if !service
        .permissions
        .contains(&ExtensionPermission::ReadDocument)
    {
        return Err(format!("扩展“{}”没有 read_document 权限", service.name));
    }
    if document.len() > MAX_INPUT_BYTES {
        return Err("当前文档超过扩展协议的 8 MiB 输入上限".to_owned());
    }
    let request = ExtensionRequest {
        protocol: PROTOCOL_VERSION,
        request_id: NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
        method: "transformDocument",
        document: ExtensionDocument {
            text: document,
            path: service
                .permissions
                .contains(&ExtensionPermission::ReadDocumentPath)
                .then_some(document_path)
                .flatten(),
        },
    };
    let bytes =
        serde_json::to_vec(&request).map_err(|error| format!("无法编码扩展请求：{error}"))?;
    let response = run_process(service, &bytes)?;
    validate_response(service, request.request_id, &response)
}

fn run_process(service: &ExtensionService, request: &[u8]) -> Result<Vec<u8>, String> {
    let working_directory = service
        .program
        .parent()
        .ok_or_else(|| "扩展程序没有父目录".to_owned())?;
    let mut command = Command::new(&service.program);
    command
        .args(&service.args)
        .current_dir(working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .env("RUPORA_EXTENSION_PROTOCOL", "1");
    for variable in ["SystemRoot", "WINDIR", "HOME", "USERPROFILE", "TMP", "TEMP"] {
        if let Some(value) = std::env::var_os(variable) {
            command.env(variable, value);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动扩展“{}”：{error}", service.name))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "无法连接扩展输入".to_owned())?;
    let request = request.to_owned();
    let (writer_sender, writer_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result = stdin
            .write_all(&request)
            .and_then(|()| stdin.write_all(b"\n"));
        let _ = writer_sender.send(result);
    });
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法连接扩展输出".to_owned())?;
    let limit = service.max_output_bytes + 1;
    let (reader_sender, reader_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(limit as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = reader_sender.send(result);
    });

    let timeout = Duration::from_millis(service.timeout_ms);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("无法等待扩展“{}”：{error}", service.name))?
        {
            break status;
        }
        if started.elapsed() >= timeout {
            child.kill().ok();
            child.wait().ok();
            return Err(format!(
                "扩展“{}”超过 {} ms 超时",
                service.name, service.timeout_ms
            ));
        }
        thread::sleep(Duration::from_millis(5));
    };
    let remaining = timeout.saturating_sub(started.elapsed());
    writer_receiver
        .recv_timeout(remaining)
        .map_err(|_| format!("扩展“{}”没有在超时前关闭输入", service.name))?
        .map_err(|error| format!("无法发送扩展请求：{error}"))?;
    let remaining = timeout.saturating_sub(started.elapsed());
    let output = reader_receiver
        .recv_timeout(remaining)
        .map_err(|_| format!("扩展“{}”没有在超时前关闭输出", service.name))?
        .map_err(|error| format!("无法读取扩展响应：{error}"))?;
    if output.len() > service.max_output_bytes {
        return Err(format!(
            "扩展“{}”响应超过 {} 字节上限",
            service.name, service.max_output_bytes
        ));
    }
    if !status.success() {
        return Err(format!("扩展“{}”退出失败：{status}", service.name));
    }
    Ok(output)
}

fn validate_response(
    service: &ExtensionService,
    request_id: u64,
    response_bytes: &[u8],
) -> Result<ExtensionInvocation, String> {
    let response: ExtensionResponse = serde_json::from_slice(response_bytes)
        .map_err(|error| format!("扩展“{}”返回无效 JSON：{error}", service.name))?;
    if response.protocol != PROTOCOL_VERSION || response.request_id != request_id {
        return Err(format!("扩展“{}”返回了错误的协议或请求 ID", service.name));
    }
    if let Some(error) = response.error {
        return Err(format!("扩展“{}”报告错误：{error}", service.name));
    }
    let result = response
        .result
        .ok_or_else(|| format!("扩展“{}”没有返回结果", service.name))?;
    if result.replacement.is_some()
        && !service
            .permissions
            .contains(&ExtensionPermission::ReplaceDocument)
    {
        return Err(format!(
            "扩展“{}”没有 replace_document 权限，拒绝其全文替换",
            service.name
        ));
    }
    Ok(ExtensionInvocation {
        replacement: result.replacement,
        message: result
            .message
            .map(|message| message.chars().take(512).collect()),
    })
}

fn validate_config(config: &ExtensionConfig) -> Result<(), String> {
    if config.services.len() > MAX_SERVICES {
        return Err(format!("扩展服务不能超过 {MAX_SERVICES} 个"));
    }
    let mut names = HashSet::new();
    for service in &config.services {
        validate_service(service)?;
        if !names.insert(service.name.to_lowercase()) {
            return Err(format!("扩展服务名称重复：{}", service.name));
        }
    }
    Ok(())
}

fn validate_service(service: &ExtensionService) -> Result<(), String> {
    if service.name.trim().is_empty() || service.name.chars().count() > 64 {
        return Err("扩展服务名称必须为 1–64 个字符".to_owned());
    }
    if !service.program.is_absolute() {
        return Err(format!("扩展“{}”必须使用绝对可执行路径", service.name));
    }
    if service.args.len() > 16
        || service.args.iter().any(|argument| argument.len() > 1_024)
        || !(100..=HARD_MAX_TIMEOUT_MS).contains(&service.timeout_ms)
        || !(1..=HARD_MAX_OUTPUT_BYTES).contains(&service.max_output_bytes)
    {
        return Err(format!("扩展“{}”的参数或资源上限无效", service.name));
    }
    Ok(())
}

const fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

const fn default_output_bytes() -> usize {
    DEFAULT_OUTPUT_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(permissions: &[ExtensionPermission]) -> ExtensionService {
        ExtensionService {
            name: "test".to_owned(),
            program: if cfg!(windows) {
                PathBuf::from(r"C:\test\extension.exe")
            } else {
                PathBuf::from("/test/extension")
            },
            args: Vec::new(),
            permissions: permissions.iter().copied().collect(),
            timeout_ms: 1_000,
            max_output_bytes: 4_096,
        }
    }

    #[test]
    fn registry_is_disabled_when_the_config_does_not_exist() {
        let directory = tempfile::tempdir().unwrap();
        let registry = ExtensionRegistry::load(directory.path().join("extensions.json")).unwrap();
        assert!(!registry.is_enabled());
        assert!(registry.services().is_empty());
    }

    #[test]
    fn rejects_relative_programs_and_unsafe_resource_limits() {
        let mut extension = service(&[ExtensionPermission::ReadDocument]);
        extension.program = PathBuf::from("extension");
        assert!(validate_service(&extension).unwrap_err().contains("绝对"));

        extension.program = if cfg!(windows) {
            PathBuf::from(r"C:\test\extension.exe")
        } else {
            PathBuf::from("/test/extension")
        };
        extension.timeout_ms = 31_000;
        assert!(validate_service(&extension).is_err());
    }

    #[test]
    fn rejects_replacement_without_an_explicit_permission() {
        let extension = service(&[ExtensionPermission::ReadDocument]);
        let response = br#"{"protocol":1,"requestId":42,"result":{"replacement":"changed"}}"#;
        let error = validate_response(&extension, 42, response).unwrap_err();
        assert!(error.contains("replace_document"));

        let extension = service(&[
            ExtensionPermission::ReadDocument,
            ExtensionPermission::ReplaceDocument,
        ]);
        let result = validate_response(&extension, 42, response).unwrap();
        assert_eq!(result.replacement.as_deref(), Some("changed"));
    }

    #[test]
    fn rejects_wrong_request_ids_and_unknown_response_fields() {
        let extension = service(&[ExtensionPermission::ReadDocument]);
        let wrong_id = br#"{"protocol":1,"requestId":41,"result":{}}"#;
        assert!(validate_response(&extension, 42, wrong_id).is_err());
        let unknown = br#"{"protocol":1,"requestId":42,"result":{},"extra":true}"#;
        assert!(validate_response(&extension, 42, unknown).is_err());
    }

    #[test]
    fn executes_a_json_service_without_a_shell_wrapper() {
        let response = r#"{"protocol":1,"requestId":42,"result":{"message":"ok"}}"#;
        let extension = platform_service(response, false);

        let bytes = run_process(&extension, b"{}").unwrap();
        let invocation = validate_response(&extension, 42, &bytes).unwrap();
        assert_eq!(invocation.message.as_deref(), Some("ok"));
    }

    #[test]
    fn terminates_a_service_that_exceeds_its_deadline() {
        let extension = platform_service("", true);
        let started = Instant::now();

        let error = run_process(&extension, b"{}").unwrap_err();

        assert!(error.contains("超时"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(windows)]
    fn platform_service(response: &str, hangs: bool) -> ExtensionService {
        let program = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        let command = if hangs {
            "$null=[Console]::In.ReadLine(); while ($true) {} ".to_owned()
        } else {
            format!(
                "$null=[Console]::In.ReadLine(); [Console]::Out.Write('{}')",
                response.replace('\'', "''")
            )
        };
        ExtensionService {
            name: "process-test".to_owned(),
            program,
            args: vec![
                "-NoProfile".to_owned(),
                "-NonInteractive".to_owned(),
                "-Command".to_owned(),
                command,
            ],
            permissions: [ExtensionPermission::ReadDocument].into_iter().collect(),
            timeout_ms: if hangs { 100 } else { 5_000 },
            max_output_bytes: 4_096,
        }
    }

    #[cfg(unix)]
    fn platform_service(response: &str, hangs: bool) -> ExtensionService {
        let command = if hangs {
            "IFS= read -r input; while :; do :; done".to_owned()
        } else {
            format!(
                "IFS= read -r input; printf '%s' '{}'",
                response.replace('\'', "'\\''")
            )
        };
        ExtensionService {
            name: "process-test".to_owned(),
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_owned(), command],
            permissions: [ExtensionPermission::ReadDocument].into_iter().collect(),
            timeout_ms: if hangs { 100 } else { 5_000 },
            max_output_bytes: 4_096,
        }
    }
}
