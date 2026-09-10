# 源码与输入审查记录（2026-09-10）

本轮以当前工作区为基线，保留此前架构重构与尚未提交的改动。审查从源码调用链建立候选，通过失败测试或原生窗口复现后修复。

## 覆盖范围

| 范围 | 审查内容与验证方式 |
| --- | --- |
| 文档、编辑、历史 | `document`、`editing`、`editor_buffer`、`wysiwyg`、`table`、`merge`、`session`；Unicode 坐标、删除、格式取消、撤销与重做 |
| 编辑器与应用 | `app`、`app_state`、`editor/*`；真实 egui 输入帧、点击焦点、方向选区、换行、Tab、模式和文档切换 |
| 渲染 | `markdown`、`rendering`、`rendering/prepared`、`native_preview`、`code_highlight`、`presentation`；代码判定、投影、目录、公式高度和资源缓存 |
| 文件与后台 | `recovery`、`export`、`workspace`、`extensions`、`instance`、`updater`、`diagnostics`、`background`、`external_changes`、`main`；资源路径、恢复预算、失败和异步结果 |
| 构建 | 根项目与 fuzz 锁文件、现有 CI 工作流、格式、测试、Clippy、发布构建和性能门禁 |

## 已复现问题与修复

- **代码正文误隐藏、删除错位**：在写作模式点击代码行 `****` 中间，星号消失；按 Delete 会把下一行合并进来。空格式标记折叠现在检查解析上下文，代码保持字面显示。双反引号包裹的单个反引号也映射到正文，而不是开围栏。
- **格式命令生成无效 Markdown**：含反引号的行内代码、包含闭合围栏的代码块、行中插入代码块，以及标签含括号的链接。生成围栏依据正文选择长度；保留原文、Unicode 选区和取消格式行为；链接保留已有 Markdown 转义与代码语义。
- **代码内网址被擅自改成链接**：源码和写作模式的智能网址粘贴现在跳过代码选区，覆盖围栏代码、行内代码和缩进代码，粘贴实际剪贴板文本。
- **点击、方向选区和连续 Tab**：源码点击立即更新真实光标；缩进保留反向选择的活动端；使用按键事件自身的 Shift 状态。连续 Tab/Shift+Tab 在创建编辑控件前按顺序执行，不再把整段选择替换成制表符或清空；两种编辑模式都与逐帧按键对照验证。
- **目录和公式渲染**：三反引号行内代码不再误入代码块逻辑；目录保护标题字面标点及完整目标，正确处理百分号、井号，跳过原始 HTML；公式使用自然高度，不再占满父区域剩余空间。
- **隐藏的活动标签**：文档激活或窗口宽度改变时滚动到活动标签，并保留用户平时手动浏览标签的能力。
- **导出图片缺失或重复**：使用与 Markdown 渲染相同的 URL 编码匹配中文、空格等图片路径；基于原 HTML 一次性替换图片资源，避免 PDF 名称碰撞导致两处图片变成同一张。SVG 加载字体和相对图片资源，并沿用文件、尺寸和总量预算；资源读取失败时明确报错。
- **恢复快照丢失旧稿**：有文档超预算时保留其上次副本，同时更新正常文档。仅在文档身份和快照校验一致时合并；身份未知或磁盘快照被替换时拒绝覆盖。已关闭或保存的文档不重新纳入旧稿，关闭标签时立即同步恢复快照。超限状态明确区分最新副本、旧副本和没有副本的文档，恢复文件格式不变。
- **单段富文本排版卡顿**：将每个样式片段从头扫描字符位置，改为按有序片段一次前向遍历 UTF-8 文本；新增文字、字体和样式片段语义回归。

## 输入测试

自动测试直接投递 egui 事件，覆盖源码与写作模式。固定种子的混合输入包含 6 类 Markdown 文档、3 个种子、每组 64 步，共 1,152 步，交替执行 Home/End、方向键与 Shift 选区、Delete/Backspace、Enter/Shift+Enter、Tab、中文及 CRLF 粘贴、撤销和模式切换；验证字符位置有效、内部换行规范，以及撤销/重做恢复完整原文。

原生 Windows 窗口已复现点击代码星号后消失及 Delete 误合并行的问题，现场保存在 `target/source-input-audit-desktop-before.md`。重新构建发布版后，使用 `target/source-input-audit-desktop-fixed.md` 实测通过：

- 点击 `****` 中间，四个星号保持可见；Delete 只删除一个星号，下一行保持独立；撤销恢复原文。
- 在代码最后一行 End、Down，焦点进入代码后的段落，文档仍为 15 行，段落和代码框位置不变。
- 普通段落末尾按一次 Enter，创建独立段落；输入“新增段落🙂”保持在代码块外，与任务列表分离；分别撤销输入和换行后恢复原文。
- 切换源码模式，全选后 Tab、Shift+Tab，正文完整保留并恢复；全选 Backspace 删除到空文档，再撤销，源码文本与测试前逐字一致。
- 返回写作模式，测试标签无未保存标记；应用保持打开。

## 验证证据

红测日志位于 `target/source-input-audit-red-*.log`。本轮新增 44 项正常执行的回归测试，另加 1 项需显式运行的发布模式性能测量。额外检查连续 Tab 后的文本和 IME 事件仍保持原顺序；测试以 egui 处理后的输入为基准，避免把框架自动设置的 repeat 标志误判为应用改写事件。

| 检查 | 结果与证据 |
| --- | --- |
| 全目标测试 | 450 passed、0 failed、3 ignored；其中库测试 428 passed，另 22 项集成/示例测试。`target/source-input-audit-all-tests-final.log` |
| Benchmark smoke | 4 项 Criterion smoke 通过，包含在全目标测试日志中 |
| Clippy | `--all-targets --all-features --locked -- -D warnings` 通过，`target/source-input-audit-clippy-final.log` |
| Fuzz 编译 | `cargo check --manifest-path fuzz/Cargo.toml --bins --locked` 通过，`target/source-input-audit-fuzz-check-final.log` |
| 依赖门禁 | cargo-deny 的 advisories、bans、licenses、sources 全部通过，`target/source-input-audit-deny.log` |
| 格式和空白 | `cargo fmt --all -- --check`、`git diff --check` 通过 |
| 发布构建 | `target/release/rupora.exe`，35,369,472 字节，2026-09-10 18:57:45 +08:00；`target/source-input-audit-release.log` |
| 大文档性能门禁 | 6 项全部通过；解析 2 万节 0.023 s、重建 2 万块 0.040 s、记录编辑 0.003 s、原子命令 0.003 s、导出 2 千节 HTML 0.030 s、替换 2.5 万处 Unicode 0.001 s。`target/source-input-audit-perf.log` |

发布程序 SHA-256：`50B97A835FE4574073CC594B7A4276E155434ADC8FE463DD506F17408D6BD458`。窗口复测使用该程序。target 文件是本地运行证据，不纳入源码。

## 富文本排版测量

显式执行 `cargo test --release --lib --locked -j 2 full_audit_measures_single_rich_paragraph -- --ignored --nocapture`，修复前后均使用生产 `wysiwyg_layout`，提前构造投影和字体上下文，预热一次后取三次样本中位数。该测试在普通全目标测试中忽略，本轮另行运行通过。数值仅表示这一合成富文本段落的排版调用，不代表应用端到端延迟。

| 样式跨度数 | Markdown 字节数 | 修复前中位数 | 修复后中位数 |
| --- | --- | --- | --- |
| 2,000 | 12,000 | 12.889 ms | 0.450 ms |
| 4,000 | 24,000 | 38.162 ms | 0.877 ms |
| 8,000 | 48,000 | 172.800 ms | 1.508 ms |
| 16,000 | 96,000 | 511.670 ms | 3.667 ms |

日志：`target/source-input-audit-rich-layout-before.log`、`target/source-input-audit-rich-layout-after.log`。最后一组耗时约为原来的 1/140。实现将样式片段的字符到字节映射从反复扫描改为一次前向遍历，保留字体、正文和片段样式行为。

本地验证环境为 Windows；没有把它等同于 Linux/macOS CI，也没有把 fuzz 编译检查等同于长时间模糊测试。源码审查和回归覆盖不能证明所有任意输入组合均无缺陷。
