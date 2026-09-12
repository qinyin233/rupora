<p align="center">
  <img src="assets/icons/128x128.png" width="96" height="96" alt="RUPORA icon">
</p>

<h1 align="center">RUPORA</h1>

<p align="center">
  <strong>原生 Rust Markdown 编辑器，专注单画布所见即所得写作。</strong>
  <br>
  Native window · Native layout · Native rendering · No WebView
</p>

<p align="center">
  <a href="https://github.com/qinyin233/rupora/actions/workflows/native-ci.yml"><img src="https://github.com/qinyin233/rupora/actions/workflows/native-ci.yml/badge.svg" alt="Native Rust CI"></a>
  <a href="https://github.com/qinyin233/rupora/releases"><img src="https://img.shields.io/badge/version-2.0.0--alpha.2-f59e0b" alt="Version 2.0.0-alpha.2"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.92%2B-000000?logo=rust" alt="Rust 1.92 or newer"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-2563eb" alt="MIT license"></a>
</p>

<p align="center">
  <a href="#为什么选择-rupora">核心能力</a> ·
  <a href="#快速开始">快速开始</a> ·
  <a href="#原生架构">原生架构</a> ·
  <a href="#参与开发">参与开发</a> ·
  <a href="docs/ROADMAP.md">路线图</a>
</p>

> [!IMPORTANT]
> RUPORA 2 当前处于 `2.0.0-alpha.2` 阶段。可从 [Releases](https://github.com/qinyin233/rupora/releases)
> 下载 Windows、macOS 和 Linux 预发布安装包；使用前请保留重要文档的独立备份。

## 为什么选择 RUPORA

RUPORA 的目标不是在浏览器编辑器外面套一层 Rust，而是让窗口、输入、Markdown 编辑、布局、
渲染、文档生命周期和导出都由原生 Rust 代码负责。

| | |
|---|---|
| **写作就是排版** | 默认使用单画布 WYSIWYG。标题、列表、引用、任务、行内代码和其他格式直接呈现最终视觉效果，不需要在源码与预览之间来回对照。 |
| **从输入到渲染都原生** | 基于 `eframe` / `egui` 和 RUPORA 自己的视觉投影、布局与命中系统；默认构建不需要 Node.js、Vue、Vditor、Tauri 或 WebView。 |
| **把文档安全放在首位** | 保留原文件编码与换行风格，提供原子保存、文件锁、外部修改三方合并、崩溃恢复和会话恢复。 |
| **面向真实桌面工作流** | 多标签、工作区、目录树、大纲、查找替换、HTML/PDF 导出、系统打印、深浅主题和跨平台安装包流水线。 |

## 功能概览

### 专注写作

- 单画布所见即所得编辑，以及一键切换的 Markdown 源码模式
- CommonMark 与 GFM 常用语法：标题、强调、删除线、链接、列表、任务、引用和代码
- GFM 表格、脚注、交叉引用、front matter、标题 ID 与自动目录
- 原生数学公式、Mermaid 图表、语法着色代码块和可视化表格编辑
- 中文 IME、Emoji、鼠标点击/拖选、键盘导航和 Markdown 智能续行

### 组织文档

- 多文档标签、最近文件、拖放打开和上次会话恢复
- 工作区目录树、文档大纲、相对文档链接和本地附件打开
- 查找/替换、常用格式命令、可配置快捷键及文档级撤销/重做
- 跨解析稳定块 ID，让前方编辑不会打乱当前活动块和组件状态

### 可靠保存

- UTF-8、UTF-8 BOM、UTF-16 LE/BE BOM、GBK/GB18030 检测与往返保存
- 保留 LF、CRLF 或 CR 换行风格，并精确跟踪未保存状态
- 原子写入、并发文件锁和外部文件变更监控
- 干净文档自动重载；未保存修改通过三方合并与持续冲突提示保护
- 带校验、损坏隔离和版本迁移的崩溃恢复快照

### 导出与扩展

- HTML/PDF 导出、系统打印，以及 PNG、JPEG、GIF、BMP、WebP、ICO、SVG 图片
- Ed25519 签名的更新清单、SHA-256 校验、CycloneDX SBOM 和构建来源证明
- 默认关闭的进程外扩展服务，具备显式权限、超时和输入/输出资源上限
- Windows、Linux、macOS CI，以及 x86_64/ARM64 六种发布目标

## 快速开始

### 1. 准备环境

安装 [Rust](https://www.rust-lang.org/tools/install) `1.92` 或更高版本。

Windows 和 macOS 不需要额外的前端工具链。Debian/Ubuntu 还需要原生窗口依赖：

```bash
sudo apt-get update
sudo apt-get install -y libgtk-3-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev
```

### 2. 获取并运行

```bash
git clone https://github.com/qinyin233/rupora.git
cd rupora
cargo run --locked
```

首次构建需要下载和编译 Rust 依赖，因此会比后续启动更久。

### 3. 构建发布版

```bash
cargo build --release --locked
```

生成的程序位于：

- Windows：`target/release/rupora.exe`
- Linux/macOS：`target/release/rupora`

Windows 的调试版和发布版都使用 GUI 子系统，直接启动 `rupora.exe` 不会额外弹出命令行窗口。

## 原生架构

RUPORA 1.x 使用 Tauri + Vue + Vditor：Rust 负责文件命令，编辑、解析和渲染核心仍运行在
WebView 中。RUPORA 2 将默认应用路径完整迁移到 Rust：

| 层级 | 2.x 实现 |
|---|---|
| 窗口、输入、控件与渲染 | `eframe` / `egui` |
| Markdown 解析 | `pulldown-cmark` |
| WYSIWYG 布局与编辑 | RUPORA `VisualProjection`、原生块组件和逐字形命中 |
| 公式、图表与图片 | RaTeX、`mermaid-svg`、`egui` 图片加载器 |
| 文档、编码、恢复和文件生命周期 | RUPORA Rust 文档模型 |
| 文件对话框 | `rfd` 原生对话框 |

```text
键盘 / IME / 鼠标
        │
        ▼
RUPORA 原生编辑与布局 ── 可逆视觉投影 ── Markdown 源码
        │                                      │
        ├── 图片 / 公式 / Mermaid              ├── 解析与大纲
        ├── 逐字形点击与选择                    ├── 保存与恢复
        └── 单画布渲染                          └── HTML / PDF 导出
```

更完整的不变量、数据流和安全边界见[原生重写架构](docs/ARCHITECTURE.md)。

## 所见即所得的边界

默认模式采用稳定块 ID、统一原生 galley 和可逆视觉投影：编辑态与排版态共用布局，视觉位置会
映射回准确的 Markdown UTF-8 边界。代码块、独立图片、公式和图表在跨块选择时作为原子内容
处理，避免产生无法恢复的半块文档。

当前仍有这些明确限制：

- 尚未提供多光标和图片拖拽缩放。
- 行内图片使用紧凑占位；独立图片块才展开为实际图片。
- 远程图片默认不自动联网加载。
- 进程外扩展提供隔离和资源限制，但不是操作系统级沙箱，只应配置可信程序。
- Windows Authenticode 与 Apple 签名/公证仍需要维护者提供外部证书。

遇到不适合视觉编辑的 Markdown，可以随时切换到源码模式。完整完成情况见
[P1–P8 路线图](docs/ROADMAP.md)。

<details>
<summary><strong>常用快捷键</strong></summary>

| 快捷键 | 功能 |
|---|---|
| `Ctrl/Cmd + N` | 新建文档 |
| `Ctrl/Cmd + O` | 打开文件 |
| `Ctrl/Cmd + Shift + O` | 打开工作区 |
| `Ctrl/Cmd + S` | 保存 |
| `Ctrl/Cmd + Shift + S` | 另存为 |
| `Ctrl/Cmd + Z` | 撤销 |
| `Ctrl/Cmd + A` | 写作模式先选择当前块，再按一次全选文档；源码模式全选文档 |
| `Ctrl + Home` / `Ctrl + End` | 写作模式定位全文首尾；加 `Shift` 扩展选区 |
| `PageUp` / `PageDown` | 写作模式跨段翻页；加 `Shift` 扩展选区 |
| `Ctrl/Cmd + Shift + Z` / `Ctrl/Cmd + Y` | 重做 |
| `Ctrl/Cmd + F` | 查找 |
| `Ctrl/Cmd + H` | 查找与替换 |
| `F3` / `Shift + F3` | 下一个 / 上一个匹配 |
| `Ctrl/Cmd + B` | 粗体 |
| `Ctrl/Cmd + I` | 斜体 |
| `Ctrl/Cmd + K` | 链接 |

</details>

## 参与开发

欢迎提交缺陷报告、功能建议、文档改进和代码贡献：

- 阅读[贡献指南](CONTRIBUTING.md)了解构建流程、代码结构和提交规范。
- 通过 [Issues](https://github.com/qinyin233/rupora/issues) 报告可复现问题或提出功能建议。
- 开始较大的修改前，建议先创建 Issue 说明目标和交互方案。
- 涉及编辑位置的修改必须覆盖中文或 Emoji；涉及保存的修改必须覆盖失败路径。

提交前运行与 CI 对齐的质量门禁：

```bash
cargo fmt --all -- --check
cargo test --all-targets --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo run --release --locked --example perf_guard
cargo deny check --hide-inclusion-graph
cargo check --manifest-path fuzz/Cargo.toml --bins --locked
```

测试体系包括单元测试、可重放 GUI/IME 场景、HTML/PDF 视觉回归、Unicode 属性测试、故障注入、
三个 libFuzzer 目标和大文档性能预算。具体说明见[质量与性能门禁](docs/QUALITY.md)。

## 打包与发布

本地安装包使用固定版本的 Cargo Packager：

```bash
cargo install cargo-packager --locked --version 0.11.8
cargo packager --release
```

Windows 也可以运行：

```powershell
./scripts/package.ps1 -Format nsis
```

推送与 `Cargo.toml` 版本一致的 `v2.*` 标签时，GitHub Actions 会为 Windows、Linux、macOS 的
x86_64/ARM64 目标构建架构命名安装包，并在全部产物验证成功后原子发布。签名密钥、平台证书、
来源证明和回滚步骤见[发布指南](docs/RELEASE.md)。

## 扩展

扩展默认关闭，并作为独立进程通过 stdin/stdout 上的一次性 JSON 协议运行。第三方动态库不会
载入编辑器地址空间；每项服务必须使用绝对程序路径，并显式申请 `read_document`、
`read_document_path` 或 `replace_document` 权限。

仓库提供最小 Rust 示例：

```bash
cargo build --locked --example extension_uppercase
```

配置格式、协议与安全边界见[扩展文档](docs/EXTENSIONS.md)。

## 文档

| 文档 | 内容 |
|---|---|
| [架构](docs/ARCHITECTURE.md) | 原生重写边界、编辑投影、文档不变量和数据流 |
| [整体评估](docs/PROJECT-ASSESSMENT-2026-09-10.md) | 源码行为评估、架构升级依据、验证和剩余限制 |
| [路线图](docs/ROADMAP.md) | P1–P8 完成情况与后续边界 |
| [质量](docs/QUALITY.md) | 测试层次、性能预算和依赖策略 |
| [发布](docs/RELEASE.md) | 多平台打包、签名、验证与回滚 |
| [扩展](docs/EXTENSIONS.md) | 进程外协议、权限和资源限制 |
| [变更记录](CHANGELOG.md) | 版本演进与已知限制 |

## License

RUPORA 基于 [MIT License](LICENSE) 发布。
