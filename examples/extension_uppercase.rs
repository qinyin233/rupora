use std::io::{self, BufRead as _};

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    protocol: u32,
    request_id: u64,
    method: String,
    document: Document,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    text: String,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    protocol: u32,
    request_id: u64,
    result: ResultBody,
}

#[derive(Serialize)]
struct ResultBody {
    replacement: String,
    message: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let line = io::stdin()
        .lock()
        .lines()
        .next()
        .ok_or("missing extension request")??;
    let request: Request = serde_json::from_str(&line)?;
    if request.protocol != 1 || request.method != "transformDocument" {
        return Err("unsupported extension protocol or method".into());
    }
    let path_note = request
        .document
        .path
        .as_deref()
        .map(|path| format!("（来源：{path}）"))
        .unwrap_or_default();
    let response = Response {
        protocol: 1,
        request_id: request.request_id,
        result: ResultBody {
            replacement: request.document.text.to_uppercase(),
            message: format!("示例扩展已转换活动文档{path_note}"),
        },
    };
    serde_json::to_writer(io::stdout().lock(), &response)?;
    Ok(())
}
