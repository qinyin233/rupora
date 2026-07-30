# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [Unreleased]

### Changed

- 将默认应用从 Tauri + Vue + Vditor 重写为 `eframe` / `egui` 原生 Rust 程序。
- Markdown 解析、大纲、统计、预览、块范围和 HTML 导出改由 Rust 实现。
- 根目录现在使用 Cargo 构建；1.x Web 实现只作为迁移对照保留。

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
- 36 项 Markdown、事务历史、稳定块、恢复、工作区和文件往返回归测试。

### Known limitations

- 混合编辑已经按块切换源码与排版，但排版字形到 Markdown 字符的精确点击映射、跨块选择和
  表格单元格编辑尚未完成。
- 1.x 的 PDF 导出、公式和图表功能尚未迁移。

## [1.1.0] - 2026-02-27

### Added

- Tauri/Vue/Vditor 版本的多文件、拖放、主题与 HTML/PDF 导出功能。
- 编辑器与侧边栏视觉样式调整。

## [1.0.0] - 2026-02-27

### Added

- 首个 Tauri 2 + Vue 3 + Vditor 版本。

[1.1.0]: https://github.com/qinyin233/rupora/releases/tag/v1.1.0
[1.0.0]: https://github.com/qinyin233/rupora/releases/tag/v1.0.0
