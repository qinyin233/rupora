# 质量与性能门禁

## 本地验证

```bash
cargo fmt --all -- --check
cargo test --all-targets --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo run --release --locked --example perf_guard
cargo deny check --hide-inclusion-graph
cargo check --manifest-path fuzz/Cargo.toml --bins --locked
```

`examples/perf_guard.rs` 使用宽松但明确的 CI 上限，防止算法意外退化：

- 解析 2 万个 Markdown 段落不超过 8 秒。
- 为包含 12.8 万个 Unicode 行内代码片段的单段落构建视觉投影不超过 2 秒。
- 在 2 万块文档中协调一次小修改不超过 1 秒。
- 在 2 万段文档中记录一次不触发同步分析的编辑不超过 250 毫秒。
- 读取这次编辑后的块索引不超过 1 秒，且不提前刷新全文统计。
- 在统计仍延迟时重复读取未变化的块索引 1000 次不超过 100 毫秒。
- 在 2 万段文档中通过原子命令接口完成一次编辑不超过 250 毫秒（含工作副本和历史提交）。
- 将 2 千段 Markdown 导出为 HTML 不超过 2 秒。
- 对包含中文和 emoji 的文本替换 2.5 万个匹配项不超过 1 秒。

这些是回归预算，不是硬件性能宣传。Criterion 基准位于 `benches/editor_core.rs`。

## 测试层次

- 单元测试验证文档、编辑、解析、合并、恢复、导出和桌面模块。
- 可重放 egui 帧验证中文 IME 与 AccessKit 编辑器语义。
- 原生 WYSIWYG 回归验证复杂块共用 galley、逐字形点击、任务框命中、原子媒体边界和链接目标显露。
- 参考文档验证 HTML 结构，并把 PDF 页转为 SVG 和像素检查空白、裁切及布局坍缩。
- Proptest 对任意 Unicode 文本验证格式往返、块索引、表格和三方合并性质。
- `tests/properties.rs` 将最小失败种子保存到 `tests/properties.proptest-regressions`；
  重跑同一测试会先回放该文件中的种子。CI 失败时保留此文件及模块级
  `proptest-regressions` 目录作为 artifact，便于复现并把确认的最小样例提交为回归。
- 故障注入验证临时文件同步之后的提交失败不会破坏原文件。
- `fuzz/` 提供 Markdown 流水线、表格解析和 WYSIWYG 投影三个 libFuzzer 目标。
- WYSIWYG 目标与 `tests/projection_fuzz.rs` 共用驱动：初始源码最多 64 KiB，
  输入最多 256 KiB，每次替换最多 4 KiB，每条轨迹最多 32 步。位置字段可覆盖全文，
  每步验证投影边界、选区及样式覆盖；普通 CI 还回放固定种子的多步轨迹。
  这些是投影测试，不代替 App、历史、系统 IME 或真实窗口验收。
- 每周及手动 CI 使用 nightly 构建并分别运行三个 fuzz 目标，每个至少分配 30 秒；本地 `cargo check` 只验证可构建性。
- 已发现的 fuzz 崩溃输入转为常规回归测试，每次 push 都会运行；fuzz 失败时保留崩溃样本 artifact 和调用栈。

关键行为契约、尚需验证的边界及任务状态入口见 [可靠性改进与验收](RELIABILITY.md)。

## 依赖策略

`deny.toml` 检查 RustSec 公告、许可证、来源和重复依赖。重复版本保持警告级别，因为图形、
字体和窗口上游有时必须并存不同主版本；漏洞、未知来源或未允许许可证会阻断 CI。
