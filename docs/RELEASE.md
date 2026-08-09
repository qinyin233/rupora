# 发布、签名与回滚

## 产物

推送 `v*` 标签后，工作流首先读取 Cargo 元数据，并要求标签与 `Cargo.toml` 中的版本完全
一致。例如版本 `2.0.0-alpha.1` 只能由标签 `v2.0.0-alpha.1` 发布。包含 SemVer
预发布标识的版本会自动创建 GitHub prerelease，稳定版本才会标记为 latest。

验证通过后，工作流会在六个原生 runner 上构建：

- Windows x86_64 / ARM64
- Linux x86_64 / ARM64
- macOS Intel / Apple Silicon

每个架构任务只会：

1. 使用锁定的 Rust 1.92、精确版本 `cargo-packager 0.11.8` 和 Cargo.lock 构建。
2. 注册 Markdown 与文本文件关联。
3. 生成 CycloneDX JSON SBOM。
4. 给安装包文件名附加平台与架构，避免 Release 中不同架构相互覆盖。
5. 上传隔离的 GitHub Actions 工作流产物，不直接修改 GitHub Release。

所有六个架构成功后，汇总任务才会读取更新清单私钥，为每个平台生成包含版本、target、
文件名、HTTPS URL、长度和 SHA-256 的 Ed25519 签名清单，并生成
`SHA256SUMS-<platform>-<arch>.txt`。最终发布任务重新验证全部校验和，使用 GitHub OIDC
创建构建来源证明，并先把全部文件上传到不可见的 draft Release；只有所有上传均成功后，
才把草稿切换为公开发布。任一架构、签名、校验或上传失败都不会产生对外可见的半成品发布。

构建任务只有仓库读取权限，且 checkout 不持久化 GitHub 凭据。Release 写权限、OIDC 和
attestation 权限只存在于最终发布任务；更新签名私钥、Windows 证书与 Apple 凭据也只注入
实际使用它们的步骤。失败后可以重新运行工作流；若上传阶段失败，未完成的内容保持为草稿。

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

汇总任务会检查私钥与公钥是否配对；客户端构建只把 `UPDATE_PUBLIC_KEY_BASE64` 嵌入
二进制。签名私钥只在生成清单的步骤中可见，不得写入 variable、源码、安装包、缓存或日志。

轮换密钥必须分两次发布，不能同时替换客户端公钥和清单签名私钥：

1. 过渡发布：保持旧的 `UPDATE_SIGNING_KEY_BASE64`，把旧公钥写入可选 variable
   `UPDATE_SIGNING_PUBLIC_KEY_BASE64`，再把 `UPDATE_PUBLIC_KEY_BASE64` 改为新公钥。
   这样过渡版本内置新公钥，但其更新清单仍由旧私钥签名，已安装的旧版本可以验证并升级。
2. 完成轮换：确认过渡版本已经发布后，把 `UPDATE_SIGNING_KEY_BASE64` 换成新私钥，并把
   `UPDATE_SIGNING_PUBLIC_KEY_BASE64` 换成新公钥（或删除该 variable，让工作流回退到
   `UPDATE_PUBLIC_KEY_BASE64`），再发布后续版本。

回滚到只信任旧公钥的客户端时，也必须使用旧私钥生成该客户端可接受的清单；不要在同一
Release 中混用两把私钥。

## 可选平台代码签名

仓库配置以下值后，发布工作流会自动启用签名：

- macOS：repository variable `APPLE_SIGNING_IDENTITY`（完整的 Developer ID Application
  身份），以及 secrets `APPLE_CERTIFICATE`、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_ID`、
  `APPLE_PASSWORD`、`APPLE_TEAM_ID`。
- Windows：`WINDOWS_CERTIFICATE_BASE64`、`WINDOWS_CERTIFICATE_PASSWORD`。

`APPLE_CERTIFICATE` 和 Windows 证书均为相应 P12/PFX 文件的 Base64 内容。私钥绝不能
提交到仓库。发布二进制会先在不含平台签名秘密的环境中构建；有签名材料时，打包步骤会
临时禁用 Cargo Packager 的重复构建钩子，避免 `build.rs` 继承证书密码或 Apple 凭据。
无平台签名材料的打包步骤即使执行重复构建，也会显式传入 `UPDATE_PUBLIC_KEY_BASE64`；
因此正式安装包和开发包都不会意外丢失内置的更新验证公钥。

Linux AppImage 构建还需要六个 repository variables，用来固定 Cargo Packager 0.11.8
会执行的上游工具（每种架构各三项）：

- `APPIMAGE_APPRUN_X86_64_SHA256`、`APPIMAGE_LINUXDEPLOY_X86_64_SHA256`、
  `APPIMAGE_PLUGIN_X86_64_SHA256`
- `APPIMAGE_APPRUN_AARCH64_SHA256`、`APPIMAGE_LINUXDEPLOY_AARCH64_SHA256`、
  `APPIMAGE_PLUGIN_AARCH64_SHA256`

这些值分别对应该架构的 `AppRun`、`linuxdeploy` 和
`linuxdeploy-plugin-appimage` 下载文件。工作流先通过 HTTPS 下载到隔离缓存，逐项核对
SHA-256 后才赋予执行权限并交给 Cargo Packager；变量缺失、格式错误或上游文件发生变化
都会让发布安全失败，绝不执行未校验的下载文件。更新这些哈希前，应在隔离环境中核对上游
项目、下载 URL 与发布来源，并通过独立渠道复算 SHA-256。

macOS 打包会把 `APPLE_SIGNING_IDENTITY` 显式传给 Cargo Packager，在打包期完成 app
签名与公证；工作流还会公证并装订 DMG，随后用 `codesign`、`spctl` 和 `stapler validate`
同时验证 app 与 DMG。Windows PFX 会临时导入当前用户证书库，工作流只把证书 thumbprint
交给 Cargo Packager，使主程序、NSIS 卸载器和安装器在构造期间完成 Authenticode 签名，
再验证公开产物的签名与 signer thumbprint，最后删除临时证书和 PFX。裸的
`target/release/rupora.exe` 不会作为 Release 资产上传。

没有 Apple 证书时，工作流会明确警告并生成带 GitHub 来源证明的开发包；没有 Windows
证书时不会伪装成 Authenticode 已签名。

## 验证与回滚

- 发布前确认 `git tag` 与 `Cargo.toml` 版本完全一致，并确认所需平台签名 secret 是否齐全。
- 下载后用平台工具计算 SHA-256，与相同 Release 中的校验文件比较。
- 可检查 `rupora-update-<target>.json` 中的长度和 SHA-256；客户端已先验证其 Ed25519 签名。
- GitHub CLI 可验证来源证明：`gh attestation verify <file> --repo qinyin233/rupora`。
- 回滚时从 Releases 安装上一版本；首次降级前保留应用数据目录中的 `recovery.json` 和
  `logs/`。文档本身始终位于用户选择的位置。
