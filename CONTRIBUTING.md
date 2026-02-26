# 贡献指南

感谢你对 RUPORA 的关注！我们欢迎任何形式的贡献。

## 🐛 报告 Bug

如果你发现了 Bug，请在 [Issues](https://github.com/qinyin233/RUPORA/issues) 页面提交，并包含以下信息：

- **操作系统**：Windows / macOS / Linux 及其版本
- **RUPORA 版本**：v1.0.0 等
- **复现步骤**：描述如何重现该问题
- **期望行为**：你期望看到的结果
- **实际行为**：实际发生了什么
- **截图**（如果有的话）

## 💡 功能建议

欢迎在 Issues 中提出新功能建议。请描述：

- 你想要什么功能
- 为什么需要这个功能
- 你期望的交互方式

## 🔧 提交代码

### 前置条件

- [Node.js](https://nodejs.org/) ≥ 18
- [Rust](https://rustup.rs/) ≥ 1.70
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/)

### 开发流程

1. **Fork** 本仓库
2. **克隆**你的 Fork：
   ```bash
   git clone https://github.com/your-username/RUPORA.git
   cd RUPORA
   ```
3. **安装依赖**：
   ```bash
   npm install
   ```
4. **创建特性分支**：
   ```bash
   git checkout -b feature/your-feature-name
   ```
5. **启动开发环境**：
   ```bash
   npm run tauri dev
   ```
6. **进行修改**，确保代码正常运行
7. **提交更改**：
   ```bash
   git add .
   git commit -m "feat: your feature description"
   ```
8. **推送分支**：
   ```bash
   git push origin feature/your-feature-name
   ```
9. **发起 Pull Request**

### 提交规范

本项目遵循 [Conventional Commits](https://www.conventionalcommits.org/) 提交规范：

| 类型 | 说明 |
|------|------|
| `feat` | 新功能 |
| `fix` | Bug 修复 |
| `docs` | 文档更新 |
| `style` | 代码格式调整（不影响逻辑） |
| `refactor` | 重构（非新功能、非 Bug 修复） |
| `perf` | 性能优化 |
| `test` | 添加或修改测试 |
| `chore` | 构建过程或辅助工具的变动 |

示例：
```
feat: add dark theme support
fix: resolve encoding issue with GBK files
docs: update README with new screenshots
```

### 项目结构

```
RUPORA/
├── src/                    # 前端 (Vue 3 + TypeScript)
│   ├── App.vue             # 主应用组件
│   ├── main.ts             # Vue 入口
│   └── assets/             # 静态资源
├── src-tauri/              # 后端 (Rust + Tauri 2)
│   ├── src/lib.rs          # 核心命令
│   ├── src/main.rs         # 入口
│   └── Cargo.toml          # Rust 依赖
├── package.json            # 前端依赖
└── vite.config.ts          # Vite 配置
```

### 代码风格

- **Rust**：遵循标准 Rust 格式（`cargo fmt`）
- **TypeScript/Vue**：使用项目的 EditorConfig 配置
- **提交信息**：使用 Conventional Commits 格式

## 📜 许可证

提交贡献即表示你同意将代码以 [MIT License](LICENSE) 进行开源。

## 💬 联系

如有问题，请在 [Issues](https://github.com/qinyin233/RUPORA/issues) 中讨论。
