# 源码行为审查与输入、渲染修复（2026-09-10）

## 范围与方法

审查对象为当前 `native/src` 的全部 20 个 Rust 模块，沿输入事件 → 选区 → 源码修改 → 历史事务 → Markdown 解析 → 可视投影 → 绘制与命中回查调用链。另核对构建入口、CI 门禁、属性测试与 fuzz 入口。依赖库不属于本次逐模块审查范围。

从源码推导候选行为，再用失败回归确认，修复后重跑同一回归。应用层复现使用真正的 egui `Context::run_ui`、`TextEdit`、键盘/鼠标事件和绘制输出；并非仅比较自制字符串模型。所有测试使用临时文档，不修改用户文稿。

| 模块 | 本轮审查重点 |
| --- | --- |
| `app.rs` | 输入分发、焦点、两种编辑器、跨块选区、点击映射、快捷键、多文档、表格窗口与模式切换 |
| `wysiwyg.rs` | UTF-8 字节与字符边界、原子范围、隐藏标记、换行、代码围栏、列表/表格投影 |
| `editing.rs`、`editor_buffer.rs` | 输入事务、续行、缩进、智能粘贴/配对、查找替换 |
| `markdown.rs`、`code_highlight.rs`、`table.rs` | 解析事件、块索引、目录/元数据、大纲、代码着色、表格序列化 |
| `native_preview.rs`、`export.rs` | 图片、公式/Mermaid、资源路径、SVG 缓存、HTML/PDF/打印路径 |
| `document.rs`、`recovery.rs`、`merge.rs` | 编码与 LF 约定、撤销/重做、文件身份、原子保存、外部修改、恢复快照 |
| `app_state.rs`、`workspace.rs`、`instance.rs` | 持久状态、目录遍历、单实例文件转交 |
| `extensions.rs`、`updater.rs`、`diagnostics.rs` | 异步结果应用、过期文档检查、错误传播与资源限制 |
| `main.rs`、`lib.rs` | 程序入口与模块接线 |

## 已复现并修复

以下对应本轮新增的 19 个 `source_audit_` 回归测试：18 个输入/渲染行为回归，另一个修复随机测试的预算误判。一个测试可覆盖同一根因的多个操作序列。

| 编号 | 触发操作与修复前结果 | 修复 |
| --- | --- | --- |
| 01 | 同帧输入“中文”再输入 `(`，或粘贴后立即输入；后一个智能输入处理覆盖前一个事件 | 智能输入重放只处理独立事件；批量事件保留 TextEdit 的顺序处理结果 |
| 02 | 源码模式在围栏代码内的 `- literal`、`1. literal`、`> literal` 后按 Enter，自动插入 Markdown 前缀 | 用解析器识别代码上下文，代码正文保持字面换行 |
| 03 | 程序设置跨段选区后输入，选区被第一块 TextEdit 截断 | 在块编辑器处理前恢复文档级跨块选区 |
| 04 | Python `total // 2 # comment` 中 `//` 后全部显示为注释 | 哈希注释语言不套用 C 风格斜线注释规则 |
| 05 | 跨块剪切后粘贴丢掉末尾空格/换行；同帧复制再输入只替换第一块 | 剪贴板保存精确源码；复制保留跨块选区并继续处理后续事件 |
| 06 | 普通粘贴 `甲\r\n乙\r丙`，内存混入 CR/CRLF | 两种编辑器在处理 Text/Paste/IME 事件前统一为 LF |
| 07 | 表格含 `\*literal\*`，打开再应用后变成斜体 | 表格字段保留行内 Markdown 转义，只处理表格管道符转义 |
| 08 | 缩进代码第一行为 `[TOC]`，预览把代码替换成目录 | 目录扩展检查整行与解析器代码范围的交集 |
| 09 | 标题 `Energy $E=mc^2$` 的大纲变成 `Energy ` | 大纲保留数学事件中的文字 |
| 10 | 同一投影包含两张表，第二个表头多出分隔符 | 新表头重置单元格计数 |
| 11 | 文档首块 `[TOC]` 不展开；阅读模式未使用目录/元数据生成逻辑 | 两种预览共享生成块处理，并排除代码中的目录标记 |
| 12 | 在较长的生成目录文字上按下鼠标，程序以目录偏移索引原文 `[TOC]` 并 panic | 生成块的选择命中使用原文首尾边界；链接点击仍使用生成内容的映射 |
| 13 | 在查找等辅助输入框中 Ctrl+Z，却撤销了正文 | 按实际 TextEdit 焦点分发撤销、重做和格式快捷键 |
| 14 | 打开表格编辑器后切换标签，再应用表格；光标被送到另一个文档 | 应用后激活表格所属文档，再恢复选区 |
| 15 | 命令面板输入命令后 Enter 无效，因为单行输入框已失去焦点 | 同时识别 Enter 导致的失焦，执行匹配命令 |
| 16 | 阅读模式点击目录后，只更新隐藏光标，页面仍停在原处 | 阅读画布消费待定位选区，把目标块滚动到可见区域，保持阅读模式 |
| 17 | 命令面板 Enter 切换到编辑模式后，正文也收到该 Enter；`KEEP` 变成 `KE\nEP` | 命令面板消费已经用于执行命令的 Enter 事件 |
| 18 | 查找框 Enter 定位 `EP` 后，正文的匹配结果被同一个 Enter 替换；`KEEP` 变成 `KE\n` | 查找框消费已经用于跳转的 Enter，正文仅更新选区 |
| 19 | 随机 Unicode 测试生成有效数学公式，其 SVG 字形超过“原文 × 128 + 4096”，导致测试误报 | 属性测试保留普通文本限制，并对 SVG 使用生产代码已有的 16 MiB 生成内容预算；新增最小失败样例 |

还删除了跨块剪贴板的多余裁剪函数，将重复的生成预览处理合并到同一入口，将表格应用逻辑独立出来便于验证。

## 验证与证据

- `cargo test --all-targets --locked`：299 通过，0 失败，2 忽略。忽略项是原有的手动大文档性能测量。本轮 19 个回归全部通过。
- `cargo clippy --all-targets --locked -- -D warnings`：通过。
- `cargo fmt --all -- --check`、`git diff --check`：通过。
- `cargo build --release --locked`：通过，最后一次构建耗时 1 分 11 秒。
- 修复前后日志位于本地 `target/source-audit-*.log`，不提交构建日志。
- 首批输入失败记录：`source-audit-before.log`、`source-audit-roundtrip-before.log`、`source-audit-cut-before.log`。
- 渲染与越界失败记录：`source-audit-render-before.log`；对应通过记录：`source-audit-render-after.log`。
- 焦点、表格标签、命令面板的失败记录分别为 `source-audit-focus-before.log`、`source-audit-table-tab-before.log`、`source-audit-command-before.log`。
- 全量结果：`source-audit-all-tests.log`；Clippy：`source-audit-clippy.log`。
- 补充的锚点可见区域回归：`source-audit-anchor-before.log` / `source-audit-anchor-after.log`。
- Enter 事件重复应用的失败记录：`source-audit-command-replay-before.log`、`source-audit-find-replay-before.log`；18 项通过记录：`source-audit-final-regressions.log`。
- 随机测试的最小失败样例为 ` ¡ $¡¡0 A00𠀀!!$ `。失败日志另存为 `source-audit-property-before.log`；修正后的 5 个属性测试/固定样例通过记录为 `source-audit-property-after.log`。生产渲染预算没有放宽。

桌面检查使用重新编译的 `target/release/rupora.exe` 和专用 `target/source-audit-desktop-20260910.md`：核对两种模式的目录/元数据、点击长目录、Python 整除着色、代码正文与复制按钮位置、块后正文。真实命令面板操作额外暴露了第 17 项，随后补充完整 UI 帧回归并修复；同源的查找框问题由第 18 项确认。

最终构建已重新打开并复测：阅读模式点击“第二节”会滚动页面；命令面板 Enter 切换回写作模式后，文档仍为 176 字符、25 行，标签没有未保存标记。应用保持打开供继续测试。

验证运行于本地 Windows。未将本地通过结果视为 GitHub 三平台 CI 或新一轮 libFuzzer 运行结果。本轮保留此前 UI/代码块修复的工作区改动，尚未提交或推送。
