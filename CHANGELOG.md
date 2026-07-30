# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [Unreleased]

### Changed

- 将默认应用从 Tauri + Vue + Vditor 重写为 `eframe` / `egui` 原生 Rust 程序。
- Markdown 解析、大纲、统计、预览、块范围和 HTML 导出改由 Rust 实现。
- 根目录现在使用 Cargo 构建；1.x Web 实现与 Node/Tauri 构建链已移出 2.x 源码树。
- 编辑热路径按需保存旧文本，并在输入空闲窗口批量刷新 Markdown 派生状态。

### Added

- 编辑、分屏、预览和块级混合编辑模式。
- 原生多文档、最近文件、拖放打开、工作区目录树、明暗主题与退出保护。
- 查找替换、Markdown 格式命令、可点击大纲和相对资源上下文。
- UTF-8/BOM、UTF-16 LE/BE BOM、GBK/GB18030 等编码读取。
- 保留原编码和换行风格的原子保存，以及外部修改冲突保护。
- 崩溃恢复快照和上次会话恢复。
- 文档级撤销/重做、连续输入合并，以及按最小变更片段存储的紧凑历史。
- 跨解析稳定块 ID，保持混合编辑活动块和 egui 控件状态。
- 两秒低开销外部变更探测、干净文档自动重载和脏文档冲突操作条。
- 三平台 CI、应用图标、`cargo-packager` 配置和标签发布工作流。
- 智能续行/成对符号/粘贴、命令面板、折叠、同步滚动和可配置快捷键。
- 外部修改三方合并、差异查看、文件锁、移动文件重新关联和恢复快照校验。
- 纯 Rust 数学公式、Mermaid、front matter、目录、可视化表格和 PDF/打印。
- 单实例文件转交、桌面文件关联、后台更新检查与轮转诊断日志。
- Criterion 性能预算、Proptest、fuzz、故障注入、cargo-deny 与 RustSec 门禁。
- 发布产物的 SHA-256、CycloneDX SBOM、GitHub 来源证明及平台签名密钥入口。
- 排版文本到 Markdown UTF-8 源码边界的映射，以及混合模式块内点击定位。
- 中文 IME/emoji GUI 重放、命名 AccessKit 编辑器节点和 PDF 像素视觉回归。

### Known limitations

- 混合编辑已经按块切换源码与排版，但排版字形到 Markdown 字符的精确点击映射和跨块富文本
  选择仍需要更深的排版编辑模型。
- 平台代码签名只有在仓库维护者配置外部证书后才能生效。

## [1.1.0] - 2026-02-27

### Added

- Tauri/Vue/Vditor 版本的多文件、拖放、主题与 HTML/PDF 导出功能。
- 编辑器与侧边栏视觉样式调整。

## [1.0.0] - 2026-02-27

### Added

- 首个 Tauri 2 + Vue 3 + Vditor 版本。

[1.1.0]: https://github.com/qinyin233/rupora/releases/tag/v1.1.0
[1.0.0]: https://github.com/qinyin233/rupora/releases/tag/v1.0.0
