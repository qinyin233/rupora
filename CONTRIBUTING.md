# 贡献指南

感谢关注 RUPORA。

## 开发环境

- Rust 1.92 或更高版本
- Windows、macOS，或带 Wayland/X11 的 Linux

默认 2.x 原生应用不需要 Node.js、Tauri CLI 或 WebView 开发环境。

## 本地开发

```bash
git clone https://github.com/qinyin233/rupora.git
cd rupora
cargo run
```

提交前请运行：

```bash
cargo fmt --all -- --check
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
```

## 代码结构

- `native/src/app.rs`：窗口、UI、命令、多文档和混合编辑交互
- `native/src/document.rs`：文档状态、编码、换行、冲突检测和原子读写
- `native/src/editing.rs`：查找替换、格式命令和字符位置映射
- `native/src/markdown.rs`：Markdown 解析、块范围、大纲、统计和 HTML
- `native/src/recovery.rs`：崩溃恢复快照
- `native/src/workspace.rs`：工作区目录树
- `assets/icons/`：桌面窗口和安装包使用的跨平台图标

## 提交要求

使用 Conventional Commits：

- `feat:` 新功能
- `fix:` 缺陷修复
- `refactor:` 不改变行为的重构
- `test:` 测试
- `docs:` 文档
- `chore:` 构建与维护

涉及文档保存的修改必须补充编码、换行、冲突或失败路径测试。涉及编辑器位置的修改必须覆盖
中文或 emoji，不能把 UTF-8 字节位置当成字符位置。

不要把分屏预览描述成完整所见即所得。当前实现边界见
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) 和 [`docs/ROADMAP.md`](docs/ROADMAP.md)。

贡献内容以项目的 MIT 许可证发布。
