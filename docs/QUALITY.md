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
- 每周 CI 使用 nightly 构建并分别运行三个 fuzz 目标，每个分配 30 秒；手动运行可将每个目标调至 30–600 秒。运行前从 `scripts/seed_fuzz_corpus.py` 生成只含合成 Markdown 的初始语料，避免随机输入长期停在无效 UTF-8 或不完整轨迹；本地 `cargo check` 只验证可构建性。
- 已发现的 fuzz 崩溃输入转为常规回归测试，每次 push 都会运行；fuzz 失败时保留崩溃样本 artifact 和调用栈。

关键行为契约、尚需验证的边界及任务状态入口见 [可靠性改进与验收](RELIABILITY.md)。

## 进程中断回归

`native/src/interrupted_save_tests.rs` 通过当前测试程序的专用子入口，调用实际的
Document 保存、另存为和 RecoveryStore 写入。两个父测试覆盖 3 条链路 × 5 个阶段；
默认标为 ignored 的子入口只由父测试带隔离夹具和管道启动，并非漏跑的产品回归。
另有一个父测试主动在子进程已就绪且持锁时失败，验证异常退出路径也会终止子进程并释放锁。

| 子进程停止位置 | 已有文档 | 新的另存为目标 | 恢复快照重写 |
| --- | --- | --- | --- |
| 临时文件创建后、写完未同步、文件同步后未替换 | 完整旧版 | 不存在 | 完整旧快照 |
| 替换完成、目录同步完成 | 完整新版 | 完整新版 | 完整新快照 |

父进程等待子进程明确确认停在该位置，检查它仍存活且持有文档锁，然后实际终止并回收它。
测试同时验证锁可重新取得、快照可加载、未命名草稿保留，以及旧快照和更新的磁盘正文
合并时不会静默覆盖任一版本。只承诺已经写入的检查点；尚未保存或写入快照的最新输入
不属于已证明可恢复的数据。

停止钩子与隔离锁目录只在 `cfg(test)` 中存在。握手最多等待 30 秒，终止后的回收最多
等待 10 秒，失败清理只针对测试创建的子进程和临时目录。生产二进制不接受测试控制参数。
本地验证环境为 Windows x86_64、NTFS；其他运行环境的文件系统类型应另行记录，不能
从操作系统名称推断。这些测试不模拟断电、写缓存丢失或设备故障；Windows 的目录同步
函数当前为空操作，其最后一个阶段只表示到达保存流程结尾。

2026-09-26 在 WSL2 Linux x86_64（6.18.33.2、测试目录 `/tmp` 为 tmpfs、Rust 1.92.0）
补充了 [#19](https://github.com/qinyin233/rupora/issues/19) 的并行复测。基线 `16cea82`
的三个 `process_termination` 父测试并行运行 20 轮失败 4 轮，串行 50 轮通过。
文档锁增加显式析构解锁后，原并行测试连续 200 轮通过，未将测试改成串行。
Unix 回归 `closing_document_releases_lock_while_a_duplicate_handle_survives` 保留同一
文件描述的副本，稳定重现关闭后恢复仍误报占用；修复后验证草稿重新关联成功，且新所有者
仍保持排他锁。该测试修复前失败、修复后通过。

原因是 [flock 的释放规则](https://man7.org/linux/man-pages/man2/flock.2.html)：
并发启动的子进程在 exec 前可能保留文件描述的副本，仅关闭父进程的句柄不足以立即释放锁。
回归通过 `File::try_clone` 固定这个窗口，避免依赖进程调度概率；进程终止矩阵仍覆盖真实子进程。
可用 `cargo test --lib --locked process_termination` 重跑原矩阵；重复此命令时保留默认并行度。

## 依赖策略

`deny.toml` 检查 RustSec 公告、许可证、来源和重复依赖。重复版本保持警告级别，因为图形、
字体和窗口上游有时必须并存不同主版本；漏洞、未知来源或未允许许可证会阻断 CI。
