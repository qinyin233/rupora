# 2026-09-10 架构解耦与回归记录

## 范围与基线

按用户的 ask-matt 请求，使用 improve-codebase-architecture 与 codebase-design 的流程，
先审查状态所有权，再实现模块边界。此次不更换 Rust/egui 技术栈、不修改文件格式或扩展协议。
本轮开始前工作区已有输入、换行和 UI 修复，均作为基线保留。

临时基线副本：`target/architecture-baseline-20260910/`。
原 App 的 78 个测试函数全部迁移或保留，没有遗漏或同名重复。全部原有断言继续运行。
架构候选 HTML：`%TEMP%/architecture-review-20260910.html`。

App 直接持有的状态字段从 47 个减少到 27 个；其余状态由相应模块拥有，所有新模块的状态字段均保持私有。

## 已完成的结构变化

- Session 独占文档集合和活动 ID，取消 App 中的文档数组与活动下标字段。
- EditorSurface 独占选区、IME、跨块拖选、分栏滚动与视图书签；各编辑视图使用当前唯一文档引用。
- 渲染与主题独立；缓存实现私有，编辑器通过测量、绘制和失效接口使用缓存。
- 后台模块独占线程与接收器；UI 处理类型化结果，不管理工作线程生命周期。
- 格式和历史操作在编辑器模块内闭合，外部副作用通过结果返回 App。
- 输入/布局测试迁到实际模块，段落与会话测试直接使用 Document + EditorSurface，避免初始化整个 App。
- App 剩余职责是窗口组合、菜单、文件/系统对话框、持久化调度和模块协调。

三个子代理分别负责会话实现及行为复核、渲染/外观与缓存回归、后台任务与编辑器接口测试。
主代理负责编辑器拆分、接口接线、回归修复及最终验证。

## 发现并修正的问题

1. 文件重载原先创建新 Document ID。改为稳定 ID 后，Session 始终能找到活动文档；重载时显式清除旧视图和缓存。
2. 离屏块仅按块 ID 命中旧高度。缓存现在校验内容摘要，等长正文变化也不能误用旧测量；重新测量覆盖同一条记录。
3. 模块提取初期把撤销排到壳层，导致 Split 先画撤销前内容。撤销已回到编辑器内，保持输入、菜单操作、历史和预览的顺序。
4. 程序化选区请求可能抢回查找/命令窗口的焦点。附属输入框取得焦点时会取消正文的待处理焦点请求。

新增 21 个回归测试，覆盖会话 ID、真实文件重载与保存、后台任务断开/重启、内容缓存失效、
文档切换中的 IME/跨块选择/历史状态、Unicode 书签及 Split 同帧绘制。

## 验证

最终完整检查（包含私有视图文件拆分后重跑）：

| 检查 | 结果 |
| --- | --- |
| `cargo test --all-targets --locked -j 2` | 369 通过，2 忽略；4 项 Criterion 冒烟成功 |
| `cargo clippy --all-targets --all-features --locked -j 2 -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` / `git diff --check` | 通过 |
| `cargo build --release --locked --bin rupora --example perf_guard -j 2` | 通过，3 分 15 秒 |
| `cargo deny check --hide-inclusion-graph` | advisory、ban、license、source 检查通过；上游重复版本仍为警告 |
| `cargo check --manifest-path fuzz/Cargo.toml --bins --locked -j 2` | 通过，仅验证 fuzz 目标编译 |

fuzz 初次锁定检查发现已有主项目依赖 `unicase` 未同步到 fuzz 锁文件；离线更新只补充该依赖一行，随后锁定检查通过。没有运行新的长期 fuzz 活动。

性能门禁：解析 20k 段 0.020 秒，协调 20k 块 0.036 秒，记录编辑 0.002 秒，
导出 2k 段 HTML 0.025 秒，替换 25k Unicode 匹配 0.001 秒，均低于仓库预算。

原生 Windows 发布版本实测（专用文件 `target/architecture-desktop.md`）：

- 成功恢复此前四个已保存标签，并从系统对话框打开专用测试文档。
- 点击普通段落，标题、代码块和后文位置保持；代码块进入编辑后复制按钮仍位于独立顶部区域。
- 普通段落末尾按一次 Enter，输入 `新增段落🙂`，新段落位于围栏外；切到源码确认代码围栏和后文完整。
- 在源码模式连续两次撤销，依次恢复文本输入与回车前正文，dirty 标记消失。
- 返回写作模式，在代码末行按 End、Down，光标进入 `AFTER CODE` 段落；行数仍为 13、文档保持干净。
- 切到其他标签再返回，正文与光标位置恢复。测试文档保持打开，供继续手测。

构建产物：`target/release/rupora.exe`，35,248,128 字节，构建完成时间 2026-09-10 17:21:00（本地时间）。
SHA-256：`DEED64DCD4385235BE2AEDDD4039AEBE7038D2FF382704DFB4C0528AC97961C2`。

验证日志位于 `target/architecture-*.log`。本轮未提交或推送代码。
