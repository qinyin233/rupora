<div align="center">

# ✨ RUPORA

**一个现代、轻量、跨平台的 Markdown 编辑器**

*Inspired by Typora · Powered by Rust*

[![Version](https://img.shields.io/badge/version-1.1.0-blue.svg)](https://github.com/qinyin233/RUPORA/releases)
[![Tauri](https://img.shields.io/badge/Tauri-v2-FFC131?logo=tauri&logoColor=white)](https://v2.tauri.app/)
[![Vue](https://img.shields.io/badge/Vue-3-4FC08D?logo=vue.js&logoColor=white)](https://vuejs.org/)
[![Rust](https://img.shields.io/badge/Rust-2021-000000?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

</div>

---

## 📖 简介

**RUPORA** (**RU**st-**PO**wered ma**R**kdown editor **A**pp) 是一款受 [Typora](https://typora.io/) 启发的所见即所得 Markdown 编辑器。它采用 **Rust + Tauri 2** 构建后端，**Vue 3 + TypeScript** 构建前端，搭载 **Vditor** 编辑器引擎，兼具原生应用的性能与 Web 技术的灵活性。

与 Electron 方案不同，RUPORA 基于系统 WebView 运行，**安装包仅约 3MB**，内存占用极低，启动飞快。

## 🎯 核心特性

### ✍️ 所见即所得编辑
- 采用 **Vditor IR（即时渲染）模式**，输入 Markdown 语法即时渲染为富文本
- 支持完整的 GFM（GitHub Flavored Markdown）语法
- 支持数学公式（KaTeX）、流程图、甘特图、思维导图等
- 内置代码高亮，支持数百种编程语言

### 📂 多文件管理
- 支持同时打开多个 Markdown 文件，通过侧边栏快速切换
- 侧边栏可折叠、可拖拽调整宽度（150px ~ 600px）
- 点击侧边栏中的文件即可切换编辑，当前编辑文件高亮显示

### 🗂️ 文档大纲导航
- 右侧自动生成文档大纲（TOC），根据标题层级实时更新
- 大纲面板可拖拽调整宽度（120px ~ 500px），方便阅读长文档

### 💾 智能文件处理
- 使用系统原生文件对话框（Open / Save），体验与操作系统一致
- **智能编码检测**：自动识别 UTF-8、UTF-8 BOM、UTF-16 LE/BE、GBK/GB18030 等编码
- 基于 `chardetng` 引擎兜底检测，确保中文、日文等多语言文件无乱码

### 🌓 主题与导出
- 亮/暗色主题一键切换，记住用户偏好
- 工具栏「更多」中支持导出 **HTML / PDF**

### 🧲 拖拽打开
- 支持从文件管理器拖拽 Markdown 文件到窗口直接打开

### ⌨️ 快捷键支持
| 快捷键 | 功能 |
|---------|------|
| `Ctrl + O` | 打开文件 |
| `Ctrl + S` | 保存文件 |

## 🛠️ 技术栈

| 层级 | 技术 | 说明 |
|------|------|------|
| **桌面框架** | [Tauri 2](https://v2.tauri.app/) | 基于系统 WebView，体积小、性能强 |
| **后端语言** | [Rust](https://www.rust-lang.org/) | 内存安全、零成本抽象、极致性能 |
| **前端框架** | [Vue 3](https://vuejs.org/) + TypeScript | 响应式 UI、Composition API |
| **编辑器引擎** | [Vditor](https://b3log.org/vditor/) | 开源 Markdown 编辑器，支持 IR/SV/WYSIWYG 三种模式 |
| **编码检测** | [encoding_rs](https://crates.io/crates/encoding_rs) + [chardetng](https://crates.io/crates/chardetng) | Mozilla 出品，工业级编码探测 |
| **文件对话框** | [tauri-plugin-dialog](https://crates.io/crates/tauri-plugin-dialog) | 原生系统对话框 |
| **构建工具** | [Vite](https://vitejs.dev/) | 极速前端构建 |

## 📦 安装

### 下载安装包

前往 [Releases](https://github.com/qinyin233/RUPORA/releases) 页面下载最新版本：

- **Windows**: `RUPORA_x.x.x_x64-setup.exe`（NSIS 安装包）或 `RUPORA_x.x.x_x64_en-US.msi`

> 📌 目前仅提供 Windows x64 版本，macOS / Linux 版本将在后续版本中支持。

### 从源码构建

#### 前置条件

- [Node.js](https://nodejs.org/) ≥ 18
- [Rust](https://rustup.rs/) ≥ 1.70
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/)

#### 构建步骤

```bash
# 1. 克隆仓库
git clone https://github.com/qinyin233/RUPORA.git
cd RUPORA

# 2. 安装前端依赖
npm install

# 3. 开发模式运行（热更新）
npm run tauri dev

# 4. 打包生产版本
npm run tauri build
```

构建产物位于 `src-tauri/target/release/bundle/` 目录下。

## 🏗️ 项目结构

```
RUPORA/
├── src/                    # 前端源码 (Vue 3 + TypeScript)
│   ├── App.vue             # 主应用组件（编辑器、侧边栏、状态栏）
│   ├── main.ts             # Vue 应用入口
│   └── assets/             # 静态资源
├── src-tauri/              # Tauri 后端 (Rust)
│   ├── src/
│   │   ├── lib.rs          # 核心命令：文件读写、编码检测、目录扫描
│   │   └── main.rs         # 应用入口
│   ├── Cargo.toml          # Rust 依赖配置
│   ├── tauri.conf.json     # Tauri 应用配置
│   ├── capabilities/       # 权限声明
│   └── icons/              # 应用图标（多尺寸 PNG + ICO）
├── public/                 # 公共静态文件
├── package.json            # 前端依赖 & 脚本
├── vite.config.ts          # Vite 构建配置
└── tsconfig.json           # TypeScript 配置
```

## 🖼️ 界面预览

> *截图即将上传...*

- **主编辑区**：中央为 Vditor 即时渲染编辑器，支持工具栏固定
- **左侧边栏**：已打开文件列表，可折叠、可拖拽调整宽度
- **右侧大纲**：文档标题大纲，支持点击跳转、拖拽调整宽度
- **底部状态栏**：显示当前文件路径，含侧边栏切换按钮

## 🗺️ 路线图

- [x] 所见即所得 Markdown 编辑（Vditor IR 模式）
- [x] 多文件打开与切换
- [x] 可折叠、可拖拽侧边栏
- [x] 可拖拽大纲面板
- [x] 智能编码检测（UTF-8 / GBK / UTF-16）
- [x] 原生文件对话框
- [x] 键盘快捷键
- [ ] 标签页（Tabs）支持
- [x] 文件拖拽打开
- [x] 主题切换（亮色 / 暗色）
- [ ] 最近文件列表
- [x] 导出为 PDF / HTML
- [ ] macOS / Linux 构建支持
- [ ] 自动保存
- [ ] 文件搜索与替换
- [ ] 插件系统

## 🤝 贡献

欢迎提交 Issue 和 Pull Request！

1. Fork 本仓库
2. 创建特性分支：`git checkout -b feature/awesome-feature`
3. 提交更改：`git commit -m 'feat: add awesome feature'`
4. 推送分支：`git push origin feature/awesome-feature`
5. 发起 Pull Request

## 📄 许可证

本项目采用 [MIT License](LICENSE) 开源协议。

---

<div align="center">

**用 Rust 的速度，书写 Markdown 的优雅。**

⭐ 如果觉得有用，请给个 Star！

</div>
