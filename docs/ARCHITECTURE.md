# RUPORA 2 原生重写架构

## 重写边界

RUPORA 1.x 的窗口由 Tauri 创建，但编辑、Markdown 解析和渲染均由 Vue/Vditor 在系统
WebView 中完成。Rust 只负责文件读写和少量系统调用，因此它是“Rust 后端的 Web 编辑器”，
不是 Rust 编辑器内核。

RUPORA 2 的默认构建入口改为原生 Rust：

```text
OS window + native input
          │
          ▼
     eframe / egui
          │
          ├── Document：内容、编码、换行、dirty、指纹和文件生命周期
          ├── editing：字符安全的选择区、查找替换与 Markdown 命令
          ├── pulldown-cmark：GFM、源码范围、块、大纲和 HTML
          ├── egui_commonmark：原生预览与非活动块排版
          ├── RecoveryStore：崩溃快照和会话
          └── Workspace：受限递归目录树
```

运行 `cargo build` 或 `cargo run` 不会调用 Node.js、Vite、Vue、Vditor、Tauri 或 WebView。
旧实现暂留在 `src/` 和 `src-tauri/` 作迁移对照；`src-tauri/icons/` 中的跨平台图标继续复用。

## 文档不变量

- `Document.content` 始终使用 Rust UTF-8 `String` 和内部 `\n`。
- 载入时记录原始编码、BOM 和换行风格；保存时恢复这些表示。
- dirty 状态由当前内容和最后一次成功保存的内容比较，不使用“一旦编辑永远为真”的标志。
- 已保存文件带有长度、修改时间和内容哈希指纹；覆盖外部修改前必须显式确认。
- 写入先落到同目录临时文件并同步，再原子替换目标。
- 另存为失败不会改变文档原路径和已保存基线。

## 混合编辑

`markdown::blocks` 使用 `pulldown-cmark` 的源码偏移范围生成顶层 `MarkdownBlock`。混合模式
只把活动块交给原生 `TextEdit`，其余块交给原生 CommonMark 渲染器。活动块更新后通过原字节
范围写回同一个 `Document`，随后重新分析全文。

查找、格式命令和大纲使用 Unicode 字符索引；块范围来自 UTF-8 字节索引。应用边界层负责二者
转换，避免中文或 emoji 破坏切片边界。

当前块的标识仍是解析时的字节起点，点击已排版块只定位到块首。稳定块 ID、精确字形映射和
跨块事务属于下一阶段，详见 [路线图](ROADMAP.md)。

## 恢复与持久化

- eframe 存储保存主题、面板、视图模式、最近文件、会话文件、活动文件和工作区。
- `RecoveryStore` 每五秒把 dirty 文档写到独立原子 JSON 快照。
- 正常退出清理恢复快照；异常退出后优先恢复快照，避免被普通会话文件覆盖。

## 发布

- `build.rs` 在 Windows 可执行文件中嵌入 ICO 和产品元数据。
- `cargo-packager` 从 Cargo 元数据读取名称、标识符、图标和许可证。
- 三平台 CI 执行格式、测试、Clippy 和 release 构建。
- `v2.*` 标签触发 Windows、Linux、macOS 安装包，并附加到 GitHub Release。
