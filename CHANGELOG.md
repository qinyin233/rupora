# Changelog

本文件记录 RUPORA 的所有版本变更，格式基于 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

## [1.0.0] - 2026-02-27

### 🎉 首次发布

#### 新增
- **所见即所得编辑** - 基于 Vditor IR（即时渲染）模式，输入 Markdown 语法即时渲染
- **多文件支持** - 可同时打开多个 Markdown 文件，通过侧边栏快速切换
- **文档大纲导航** - 右侧自动生成标题大纲，支持点击跳转
- **智能编码检测** - 自动识别 UTF-8、UTF-8 BOM、UTF-16 LE/BE、GBK/GB18030 等编码
- **原生文件对话框** - 使用系统原生的打开/保存对话框
- **可折叠侧边栏** - 侧边栏可折叠，支持拖拽调整宽度（150px ~ 600px）
- **可拖拽大纲面板** - 大纲面板支持拖拽调整宽度（120px ~ 500px）
- **快捷键支持** - `Ctrl+O` 打开文件，`Ctrl+S` 保存文件
- **GFM 语法支持** - 完整支持 GitHub Flavored Markdown
- **代码高亮** - 内置代码语法高亮，支持数百种编程语言
- **数学公式** - 支持 KaTeX 数学公式渲染

#### 技术
- 使用 Tauri 2 + Rust 构建后端
- 使用 Vue 3 + TypeScript 构建前端
- 使用 Vditor 作为编辑器引擎
- 使用 encoding_rs + chardetng 进行编码检测
- Windows x64 安装包（NSIS + MSI）

[1.0.0]: https://github.com/qinyin233/RUPORA/releases/tag/v1.0.0
