<div align="center">

# RUPORA 2

**一个真正以 Rust 作为应用与编辑器内核的原生 Markdown 编辑器**

Native window · Native rendering · No WebView

</div>

> 当前版本：`2.0.0-alpha.1`。这是从 1.x WebView/Vditor 版本迁移出的可运行原生版本，
> 不把“使用 Rust 后端”冒充成“Rust 编辑器重写”。

## 重写边界

RUPORA 1.x 使用 Tauri + Vue + Vditor：文件命令由 Rust 完成，但编辑、解析和渲染核心仍是
WebView 中的 JavaScript 组件。RUPORA 2 的默认构建不再调用 Node.js、Vite、Vue、Vditor、
Tauri 或 WebView。

| 层级 | 2.x 实现 |
|---|---|
| 窗口、输入、控件与渲染 | `eframe` / `egui` |
| Markdown 解析 | `pulldown-cmark` |
| Markdown 原生预览 | `egui_commonmark`、RaTeX、`mermaid-svg` |
| 文档、编码、恢复与文件生命周期 | Rust |
| 文件对话框 | `rfd` 原生对话框 |

详细设计见 [原生重写架构](docs/ARCHITECTURE.md)，完成情况和后续边界见
[路线图](docs/ROADMAP.md)。

## 已实现

- 多文档打开、切换、关闭、最近文件、拖放打开和会话恢复
- 编辑、分屏、预览，以及“当前块源码 + 其余块排版”的原生混合模式
- GFM 表格、任务列表、删除线、脚注、交叉引用、front matter、标题大纲和相对图片
- 纯 Rust 数学公式与 Mermaid 图表渲染，以及可视化表格单元格编辑
- 可点击大纲、工作区目录树、相对文档链接和本地附件打开
- 查找/替换、常用 Markdown 格式命令及快捷键
- 文档级撤销/重做；连续输入自动合并，格式化和替换保持独立历史步骤
- UTF-8、UTF-8 BOM、UTF-16 LE/BE BOM、GBK/GB18030 检测与往返保存
- 保存时保留原文件编码与 LF / CRLF / CR 换行风格
- 精确未保存状态、原子保存、文件锁、外部修改三方合并与移动文件重新关联
- 外部文件变更监控：干净文档自动重载，脏文档显示差异和持续冲突提示
- 带校验与损坏隔离的崩溃恢复快照、自动恢复和上次会话恢复
- HTML/PDF 导出、系统打印、浅色/深色主题、窗口和可执行文件图标
- 跨解析稳定的块 ID，避免前方编辑导致活动块和控件状态错位
- 大文档输入期间延迟全量派生分析，编辑缓冲只在真实修改时惰性捕获历史正文
- 排版文本到 Markdown UTF-8 边界的源码映射，支持混合模式块内点击精确定位
- 单实例文件转交、文件关联、Ed25519 签名更新检查和轮转诊断日志
- 默认关闭的进程外扩展服务，以及权限、超时、输入/输出上限和过期结果保护
- 中文 IME、Emoji、AccessKit、HTML/PDF 视觉回归、属性、fuzz 和大文档性能测试
- 六种原生平台/架构 CI，发布产物带签名清单、SBOM、校验和与来源证明

## 快捷键

| 快捷键 | 功能 |
|---|---|
| `Ctrl/Cmd + N` | 新建 |
| `Ctrl/Cmd + O` | 打开文件 |
| `Ctrl/Cmd + Shift + O` | 打开工作区 |
| `Ctrl/Cmd + S` | 保存 |
| `Ctrl/Cmd + Shift + S` | 另存为 |
| `Ctrl/Cmd + Z` | 撤销 |
| `Ctrl/Cmd + Shift + Z` / `Ctrl/Cmd + Y` | 重做 |
| `Ctrl/Cmd + F` | 查找 |
| `Ctrl/Cmd + H` | 查找与替换 |
| `F3` / `Shift + F3` | 下一个 / 上一个匹配 |
| `Ctrl/Cmd + B` | 粗体 |
| `Ctrl/Cmd + I` | 斜体 |
| `Ctrl/Cmd + K` | 链接 |

## 构建与运行

需要 Rust 1.92 或更高版本：

```bash
cargo run
```

验证：

```bash
cargo fmt --all -- --check
cargo test --all-targets --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo run --release --locked --example perf_guard
cargo deny check --hide-inclusion-graph
cargo build --release --locked
```

发布程序位于 `target/release/rupora`，Windows 下为 `rupora.exe`。

## 安装包

安装固定版本打包器：

```bash
cargo install cargo-packager --locked --version 0.11.8
cargo packager --release
```

Windows 也可运行：

```powershell
./scripts/package.ps1 -Format nsis
```

产物写入 `target/release`。推送 `v2.*` 标签时，GitHub Actions 会为 Windows、Linux 和
macOS 的 x86_64/ARM64 六种目标分别构建架构命名安装包，并附加 Ed25519 更新清单、
SHA-256 校验、CycloneDX SBOM 与构建来源证明。

正式发布前需要配置：

- secret `UPDATE_SIGNING_KEY_BASE64`：随机 32 字节 Ed25519 私钥的 Base64。
- variable `UPDATE_PUBLIC_KEY_BASE64`：运行
  `cargo run --locked --example update_public_key` 从同一私钥派生的公钥。
- 可选的 Windows Authenticode 和 Apple 签名/公证证书。

签名、验证和回滚流程见 [发布指南](docs/RELEASE.md)。

## 扩展

扩展默认关闭，并作为独立进程通过 stdin/stdout 上的一次性 JSON 协议运行，不会把第三方
动态库载入编辑器地址空间。可从“扩展 → 打开扩展配置”创建 `extensions.json`，然后为每个
绝对程序路径明确授予 `read_document`、`read_document_path` 或 `replace_document` 权限。

仓库包含一个最小 Rust 示例：

```bash
cargo build --locked --example extension_uppercase
```

协议、配置和安全边界见 [扩展文档](docs/EXTENSIONS.md)。进程隔离并不等同于操作系统沙箱，
只应配置可信程序。

## 项目结构

```text
native/src/
├── app.rs         # 原生 UI、窗口、命令、多文档与混合编辑交互
├── app_state.rs   # 持久状态、快捷键与应用命令
├── diagnostics.rs # 轮转运行日志与 panic 回溯
├── document.rs    # 文档模型、编码、换行、冲突检测与原子写入
├── editing.rs     # 查找替换、格式命令和字符位置映射
├── editor_buffer.rs # egui 编辑缓冲、惰性历史快照与 IME 适配
├── export.rs      # 原生 PDF 与打印
├── extensions.rs  # 默认关闭的进程外扩展服务协议
├── instance.rs    # 单实例协调和文件转交
├── markdown.rs    # GFM、公式、图表、块范围、大纲、统计和 HTML
├── merge.rs       # 外部修改三方合并
├── native_preview.rs # 数学、Mermaid 与 SVG 的原生预览和有界缓存
├── recovery.rs    # 崩溃恢复快照
├── source_map.rs  # 排版文本到 Markdown 源码的 Unicode 安全映射
├── table.rs       # 表格解析与可视化编辑模型
├── updater.rs     # Ed25519 签名的目标架构更新清单验证
├── workspace.rs   # 工作区目录树
├── lib.rs
└── main.rs

assets/icons/      # 桌面窗口与安装包使用的跨平台图标
```

## 仍需诚实说明的限制

P1–P8 已逐项完成，但混合模式仍不是 Typora 的像素级复制：块内点击已映射到对应源码附近，
尚未共享 CommonMark 渲染器的逐字形排版坐标；跨块富文本选择和多光标仍属于后续编辑器研究。
平台代码签名还必须由维护者提供外部证书，源码不能生成可信身份。扩展服务是进程隔离而非
OS 沙箱，只应配置可信程序。完整状态见 [P1–P8 路线图](docs/ROADMAP.md)，质量门禁见
[质量说明](docs/QUALITY.md)，扩展边界见 [扩展协议](docs/EXTENSIONS.md)。

## License

[MIT](LICENSE)
