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
| Markdown 原生预览 | `egui_commonmark` |
| 文档、编码、恢复与文件生命周期 | Rust |
| 文件对话框 | `rfd` 原生对话框 |

详细设计见 [原生重写架构](docs/ARCHITECTURE.md)，完成情况和后续边界见
[路线图](docs/ROADMAP.md)。

## 已实现

- 多文档打开、切换、关闭、最近文件、拖放打开和会话恢复
- 编辑、分屏、预览，以及“当前块源码 + 其余块排版”的原生混合模式
- GFM 表格、任务列表、删除线、脚注、标题大纲和相对图片
- 可点击大纲、工作区目录树、相对文档链接和本地附件打开
- 查找/替换、常用 Markdown 格式命令及快捷键
- UTF-8、UTF-8 BOM、UTF-16 LE/BE BOM、GBK/GB18030 检测与往返保存
- 保存时保留原文件编码与 LF / CRLF / CR 换行风格
- 精确未保存状态、原子保存、外部修改冲突保护
- 崩溃恢复快照、自动恢复和上次会话恢复
- HTML 导出、浅色/深色主题、窗口和可执行文件图标
- 26 项 Rust 回归测试、三平台 CI 和三平台发布打包工作流

## 快捷键

| 快捷键 | 功能 |
|---|---|
| `Ctrl/Cmd + N` | 新建 |
| `Ctrl/Cmd + O` | 打开文件 |
| `Ctrl/Cmd + Shift + O` | 打开工作区 |
| `Ctrl/Cmd + S` | 保存 |
| `Ctrl/Cmd + Shift + S` | 另存为 |
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
cargo clippy --all-targets --locked -- -D warnings
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

产物写入 `target/release`。推送 `v2.*` 标签时，GitHub Actions 会在 Windows、Linux 和
macOS 分别构建安装包并附加到对应 Release。

## 项目结构

```text
native/src/
├── app.rs         # 原生 UI、窗口、命令、多文档与混合编辑交互
├── document.rs    # 文档模型、编码、换行、冲突检测与原子写入
├── editing.rs     # 查找替换、格式命令和字符位置映射
├── markdown.rs    # GFM 解析、块范围、大纲、统计和 HTML 导出
├── recovery.rs    # 崩溃恢复快照
├── workspace.rs   # 工作区目录树
├── lib.rs
└── main.rs

src/               # 1.x Vue/Vditor 旧实现，仅作迁移对照
src-tauri/         # 1.x Tauri 旧实现，仅作迁移对照和复用图标
```

## 仍需诚实说明的限制

混合模式已经实现块级“当前块显示源码、其他块排版”，但还不是 Typora 的完全等价实现：
点击排版块会把光标放到块首，尚未把排版字形的精确点击位置映射回 Markdown 标记中的字符；
表格的单元格内编辑、跨块选择与统一撤销也仍需专用编辑模型。公式、图表和 PDF 导出尚未迁移。

## License

[MIT](LICENSE)
