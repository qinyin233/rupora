# 发布、签名与回滚

## 产物

推送 `v2.*` 标签后，三平台工作流会：

1. 使用锁定的 Rust 1.92、`cargo-packager 0.11.8` 和 Cargo.lock 构建。
2. 注册 Markdown 与文本文件关联。
3. 生成 CycloneDX JSON SBOM。
4. 为安装包和 SBOM 生成 `SHA256SUMS-<platform>.txt`。
5. 使用 GitHub OIDC 为发布文件创建构建来源证明。
6. 上传工作流产物并附加到 GitHub Release。

应用内更新检查只读取 GitHub HTTPS Release 元数据并打开发布页，不在运行中覆盖可执行文件。
用户可以先验证哈希和来源证明，再运行平台安装程序。这也保留了平台原生回滚路径：重新安装
上一版本，文档和恢复数据不会被卸载器删除。

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
- GitHub CLI 可验证来源证明：`gh attestation verify <file> --repo qinyin233/rupora`。
- 回滚时从 Releases 安装上一版本；首次降级前保留应用数据目录中的 `recovery.json` 和
  `logs/`。文档本身始终位于用户选择的位置。
