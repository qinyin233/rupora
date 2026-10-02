# 引用链接的辅助文本与阅读节点

追踪：[GitHub #37](https://github.com/qinyin233/rupora/issues/37)。
修复前源码为 `a63d8e0ae5abd5bfd7f332227a19f9a5ec4546e6`。

## 复现与原因

2026-10-03，实际 Windows `10.0.26300` x86_64，release 程序 SHA-256
`2000c49c24fb2e6de2323e33c6da0f9b4595ebe6b3e15b5624ff9686f4e22356`。
夹具内容：

```markdown
前[&#93;中🙂b][a🙂b]尾

[a🙂b]: https://example.com
```

画面正确显示 `前]中🙂b尾`，但 Windows 辅助功能树暴露
`前[]中🙂b][a🙂b]尾`，窗口 `4915762` 的文本节点 `76`。
文件 SHA-256 为 `0cdc1b6d60081ab0609a04f10e7a146e75b67488784faa128a3a648d8af2d6ed`。

逐块辅助文本使用没有文档引用定义的 `VisualProjection::from_markdown`，
而画面使用 `DocumentRenderView` 中的定义。引用定义在另一块时，辅助文本把链接语法
当成普通文字。真实剩余正文回归修复前失败：期望 `\n前]中🙂b尾`，实际为
`\n前[]中🙂b][a🙂b]尾`；日志 `target/issue37-accessibility-red.log`，退出码 101。

另一个真实 `EditorSurface` / AccessKit 回归证明，新建 Preview 帧没有段落文本节点。
原生文字通过 `Label::layout_in_ui` 绘制，该入口不执行 `Label::ui` 的语义注册；
Preview 也没有补充注册。日志 `target/issue37-preview-accesskit-red.log`，退出码 101。

## 修复

- 辅助文本、非活动写作块、活动编辑器的剩余正文使用同一渲染视图已有的引用定义。
  每块只克隆 `Arc`，不重复分析完整文档，不改写正文。
- 阅读块的 `NativePointerMapping::Text` 响应注册辅助文本。原子图片、数学等组件
  保留自己的语义节点；活动源码编辑和 Unicode 选区转换没有改动。
- 辅助块文本在 `widget_info` 回调内按需生成，避免未启用辅助功能且无相关事件的
  普通帧提前重建投影；egui 的辅助节点、输出事件和调试语义仍调用同一生成逻辑。
- 剩余正文回归覆盖实体、中文/emoji、格式化标签、显式/折叠/快捷引用、行内链接和
  未解析引用。实际 EditorSurface 测试检查 Preview、Hybrid、Split 的 AccessKit 文本，
  并断言文档正文不变。

## 验证

`cargo fmt --all` 后，`cargo test --lib --locked -j 1 accessible -- --nocapture`
通过 4 项，0 失败，退出码 0；日志 `target/issue37-accessibility-green.log`。
- 最终按需回调源码 `cargo fmt --all -- --check` 通过。
- `cargo test --all-targets --locked -j 1 --quiet` 通过：743 项通过、0 失败、4 项忽略；
  `target/issue37-all-tests.log`，全部 29 组通过。
- `cargo clippy --all-targets --all-features --locked -j 1 -- -D warnings` 通过，
  `target/issue37-clippy.log`，16.55 秒。

- `cargo run --release --locked -j 1 --example perf_guard` 的全部 10 项预算通过，
  `target/issue37-perf.log`。隐藏链接标签大输入为 0.096 秒，投影为 0.037 秒，
  2 万段分析 0.020 秒，其余项目均在预算内。

- `cargo build --release --locked -j 1` 通过，退出码 0，5 分 17 秒；
  `target/issue37-release.log`。验证期间源码未修改，未使用增量编译，Cargo 并发为 1。

精确源码 CI/fuzz 尚待完成。
旧提交的绿色 CI 不代替这些验证；#37 在证据补齐前保持开放。

## 原生验收

直接运行本次生成的 `target/release/rupora.exe`，没有另存一份旧程序。
35,675,136 字节，Authenticode `NotSigned`，SHA-256：
`6a271b18d83e4db528246f8e38557b7ecf26fa94538de4fee6d087448bbf2db3`。
环境仍为实际 Windows `10.0.26300` x86_64，夹具复用上述 optimized 目录中已保存的文档。

窗口 `985382` 实际恢复该文档路径与正文。活动写作编辑块仍暴露原始 Markdown，符合
源码显露语义。切换阅读后，辅助文本节点 `71` 为 `前]中🙂b尾`，与画面相同。
回到写作并 Escape 折叠编辑块后，辅助文本节点 `76` 同样正确；分栏的节点 `79`
显示同样正文，而左侧编辑区仍保留原始 Markdown 及引用定义。

正常关闭并确认进程消失，从同一路径重新启动得到窗口 `1051410`，仍恢复相同夹具。
再次切换阅读，新进程的文本节点 `72` 为 `前]中🙂b尾`，画面和链接样式正确。
正常结束验收后无 RUPORA/Cargo/rustc 残留进程。全过程文件 SHA-256 仍为
`0cdc1b6d60081ab0609a04f10e7a146e75b67488784faa128a3a648d8af2d6ed`，程序哈希也未变。
截图和真实 Windows 辅助功能树保留在本线程 Computer Use 工具输出。

这验证本夹具的原生辅助文本、源码显露和正常重启；没有运行屏幕阅读器产品，也没有
补齐安装、平台、系统输入法或崩溃恢复矩阵，这些后续工作仍由 #6 跟踪。

## Standards

独立复审固定基线 `a63d8e0`，未发现硬规范违反或可行动的维护性问题。
引用定义来自一致的渲染视图；辅助文本仍属于 rendering 模块职责。
Text 映射补充节点不会重复 `Label::layout_in_ui` 未注册的语义，保留只读/禁用状态。
按需回调同步捕获当前视图，不访问 `Context`，独立核对确认没有重入锁问题。

## Spec

独立复审未发现已确认的实现缺陷或范围扩张。当前视图的引用上下文与可视预览一致，
解析器自身的局部定义仍优先；活动编辑内容未被统一剥离 Markdown。
复审时完整门禁和原生检查尚待完成；本地进展见上节。AccessKit 节点测试证明文本存在，
不能据此声称真实屏幕阅读器或完整安装/输入法矩阵通过；这些仍由 #6 跟踪。
