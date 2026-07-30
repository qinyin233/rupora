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
- 在 2 万块文档中协调一次小修改不超过 1 秒。
- 将 2 千段 Markdown 导出为 HTML 不超过 2 秒。

这些是回归预算，不是硬件性能宣传。Criterion 基准位于 `benches/editor_core.rs`。

## 测试层次

- 单元测试验证文档、编辑、解析、合并、恢复、导出和桌面模块。
- Proptest 对任意 Unicode 文本验证格式往返、块索引、表格和三方合并性质。
- 故障注入验证临时文件同步之后的提交失败不会破坏原文件。
- `fuzz/` 提供 Markdown 流水线和表格解析两个 libFuzzer 目标。
- 每周及手动 CI 使用 nightly 构建 fuzz 目标。

## 依赖策略

`deny.toml` 检查 RustSec 公告、许可证、来源和重复依赖。重复版本保持警告级别，因为图形、
字体和窗口上游有时必须并存不同主版本；漏洞、未知来源或未允许许可证会阻断 CI。
