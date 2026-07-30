# 发布、签名与回滚

## 产物

推送 `v2.*` 标签后，发布工作流会在六个原生 runner 上构建：

- Windows x86_64 / ARM64
- Linux x86_64 / ARM64
- macOS Intel / Apple Silicon

每个架构任务会：

1. 使用锁定的 Rust 1.92、`cargo-packager 0.11.8` 和 Cargo.lock 构建。
2. 注册 Markdown 与文本文件关联。
3. 生成 CycloneDX JSON SBOM。
4. 给安装包文件名附加平台与架构，避免 Release 中不同架构相互覆盖。
5. 生成包含版本、target、文件名、HTTPS URL、长度和 SHA-256 的 Ed25519 签名清单。
6. 为安装包、SBOM 和签名清单生成 `SHA256SUMS-<platform>-<arch>.txt`。
7. 使用 GitHub OIDC 为发布文件创建构建来源证明。
8. 上传工作流产物并附加到 GitHub Release。

应用内更新检查只接受与当前 Rust target 完全匹配并通过内置公钥验证的清单。缺少公钥、
缺少清单、签名无效、版本/架构不一致都会安全失败；应用仍只打开发布页，不在运行中覆盖
可执行文件。用户可以先验证哈希和来源证明，再运行平台安装程序。这也保留了平台原生回滚
路径：重新安装上一版本，文档和恢复数据不会被卸载器删除。

## 更新清单密钥

更新清单签名与 Windows/macOS 平台代码签名是不同的信任层。首次配置时生成随机 32 字节
Ed25519 私钥，例如使用 `openssl rand -base64 32`，然后：

1. 把私钥 Base64 保存为仓库 secret `UPDATE_SIGNING_KEY_BASE64`。
2. 在本地把同一值放入 `RUPORA_UPDATE_SIGNING_KEY`，运行
   `cargo run --locked --example update_public_key`。
3. 把输出的公钥 Base64 保存为仓库 variable `UPDATE_PUBLIC_KEY_BASE64`。

发布任务会检查私钥与公钥是否配对；客户端构建把公钥嵌入二进制。签名私钥不得写入 variable、
源码、安装包、缓存或日志。轮换密钥时必须先发布一个内置新公钥的过渡版本。

## 可选平台代码签名

仓库密钥中配置以下值后，发布工作流会自动启用签名：

- macOS：`APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_ID`、
  `APPLE_PASSWORD`、`APPLE_TEAM_ID`。
- Windows：`WINDOWS_CERTIFICATE_BASE64`、`WINDOWS_CERTIFICATE_PASSWORD`。

`APPLE_CERTIFICATE` 和 Windows 证书均为相应 P12/PFX 文件的 Base64 内容。私钥绝不能
提交到仓库。没有 Apple 证书时，工作流会明确警告并生成带 GitHub 来源证明的开发包；
没有 Windows 证书时不会伪装成 Authenticode 已签名。

## 验证与回滚

- 下载后用平台工具计算 SHA-256，与相同 Release 中的校验文件比较。
- 可检查 `rupora-update-<target>.json` 中的长度和 SHA-256；客户端已先验证其 Ed25519 签名。
- GitHub CLI 可验证来源证明：`gh attestation verify <file> --repo qinyin233/rupora`。
- 回滚时从 Releases 安装上一版本；首次降级前保留应用数据目录中的 `recovery.json` 和
  `logs/`。文档本身始终位于用户选择的位置。
