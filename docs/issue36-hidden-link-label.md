# #36：隐藏链接标签中的标点编辑

跟踪：[GitHub #36](https://github.com/qinyin233/rupora/issues/36)。
修复前基线：`5a2e4ade3101699f5af964c8d218619425b78b1e`；产品源码自 `2a734f2` 未变。

## 复现与修复

`[a](u)` 的视觉标签 `a` 被替换为 `[` 后，旧编辑路径生成 `[[](u)`。
可见括号已不属于链接；解析器仍可能返回同目标的空链接，因此只检查 URL 不足。
修复先检查链接的目标、标题和完整源码范围，再检查未选择的文字及样式。
只有隐藏标签的标点输入损坏这些语义时才进行保护；显露源码、完整删除和跨边界编辑
继续使用已有路径。

新增编码保留完整代码、Math、图片、InlineHtml 和 Markdown 转义对，并使用本块引用定义
优先、文档定义后备的解析上下文。旧字面反斜杠不能吞掉新实体的 `&` 或新转义；补齐其
配对时，同步移动完整 Unicode 字符边界表，保持返回选区和后续编辑位置。

独立审查发现的空链接、Math、引用图片、混合合法转义和旧反斜杠邻接问题均先由真实
投影/编辑回归确认失败，再修复。测试检查可见文字、链接范围、目标和标题、保留样式、
中文/emoji、实体、奇偶反斜杠、插入/替换、选区、光标及连续输入。

## 本地验证与独立审查

Windows 本地 Rust `1.93.1`：

- `cargo fmt --all -- --check` 通过。
- `cargo test --all-targets --locked -j 1 --quiet` 通过：740 项通过、0 失败、4 项忽略。
  日志 `target/issue36-all-tests.log`。`link_followup` 的 11 项集成回归及块外引用定义
  的单元回归均包含在全量验证中。
- `cargo clippy --all-targets --locked -j 1 -- -D warnings` 通过；日志 `target/issue36-clippy.log`。
- `cargo build --release --locked -j 1` 通过，退出码 0，17 分 05 秒；
  日志 `target/issue36-release.log`。
- 2026-10-03，`cargo run --release --locked -j 1 --example perf_guard` 通过，退出码 0；
  日志 `target/issue36-perf.log`。128k 行内代码投影 0.051 秒、20k 节分析 0.024 秒、
  20k 块对齐 0.044 秒、更新后读取块 0.040 秒、2k 节 HTML 渲染 0.024 秒；
  其余四项预算也通过。本轮关闭增量编译以减少构建缓存。
- 规范审查未发现硬违反；空格编码的少量重复被记录为非阻断的可选整理项。
- 需求审查在最终增量中未发现实质缺陷或高置信候选；不据此推断没有未知问题。
- 新增 6 个隐藏标签编辑 fuzz 种子；语料生成成功，所见即所得目标现有 30 个合成种子。
  这不是 libFuzzer 运行结果。

提交后的精确源码 CI/fuzz 尚待最终验证。普通 push 的 fuzz 跳过不算通过。

## 原生旧版失败复现

2026-10-01，Computer Use 操作既有本地 release：
`target/acceptance-issue35-20260930/rupora.exe`，产品源码 `2a734f2`；没有运行下载的安装包。
环境为 Windows 11 家庭中文版 `10.0.22631` x86_64，版本 `2.0.0-alpha.4`，未签名。
程序 35,653,120 字节，SHA-256：
`8c65286b66470097ce1f4f248f1e1150a9c3f8f6b922af9e65a64376e5228ddd`。
本次使用字面文本和普通快捷键，不计为系统输入法组合验收。
新合成文件 `target/acceptance-issue36-20261001/hidden-label-bracket.md`：

```markdown
前[a🙂b][]尾

[a🙂b]: https://example.com
```

文件为 UTF-8 无 BOM，包含末尾换行；初始 SHA-256：
`ee8977622c2870d47fc753f385c53a24e00cf112110a06e6aa332600d71eaef7`。

写作视图聚焦正文，Home、Right、Shift+Right 仅选择 `a`，输入字面 `]`，Ctrl+S 后
首行确为 `前[]🙂b][a🙂b]尾`，显示重复标签。失败文件 SHA-256：
`d106423a6da66845ffd49d021e0d30f77f2687d60d5707d7ff195769b58ee8e8`。
Ctrl+Z、Ctrl+S 恢复初始哈希，正常退出并确认窗口消失。
截图位于本线程 Computer Use 输出；主窗口 `23071784`，未另存图片。

## 修复版原生复测

2026-10-03，Computer Use 操作上述 release 构建的独立副本：
`target/acceptance-issue36-20261003/rupora.exe`，35,674,624 字节，版本仍为
`2.0.0-alpha.4`，Authenticode `NotSigned`。SHA-256：
`33e34f49a6618319d3406e624e44a823063256d14e3dc3156ad102d8d6bf9d82`。
本轮实际查询的系统为 Windows 11 家庭中文版 `10.0.26300` x86_64；
这与 10 月 1 日旧版复现记录的系统版本不同。程序对应本次未提交修复，
Rust 源文件在全量测试及 release 构建后没有修改。

新合成文件 `target/acceptance-issue36-20261003/hidden-label-bracket.md`
使用与旧版相同的初始字节，SHA-256 为 `ee897762…eaef7`（完整值见上节）。
在系统打开对话框打开后，写作模式聚焦正文，通过 Home、Right、Shift+Right
仅选择 `a`，输入字面 `]`，Ctrl+S 后磁盘为：

```markdown
前[&#93;🙂b][a🙂b]尾

[a🙂b]: https://example.com
```

Escape 结束源码显露后，可见文字为 `前]🙂b尾`，替换标签仍有链接样式，
没有旧版的额外标签。文件 SHA-256：
`e5837703514459dfc34a771747f35177cd490bfab935f3eff245ad4edfde11cc`。
一次 Ctrl+Z、Ctrl+S 恢复初始完整哈希；一次 Ctrl+Y、Ctrl+S 恢复上述修复结果哈希。

在返回的光标位置继续输入字面中文 `中`，磁盘首行精确为
`前[&#93;中🙂b][a🙂b]尾`；写作失焦显示和阅读视图均为 `前]中🙂b尾`。
最终文件 SHA-256：
`0cdc1b6d60081ab0609a04f10e7a146e75b67488784faa128a3a648d8af2d6ed`。
正常退出后进程消失；重新启动同一个程序，恢复到同一路径，写作渲染和磁盘哈希
仍一致。随后正常退出。主窗口分别为 `789614`、`396932`；截图在本线程工具输出，
未另外导出。

本轮是实际原生窗口、普通快捷键及字面 Unicode 输入，不计为系统 IME 组合验收；
运行对象是本地编译程序，没有执行安装包。系统组合输入、安装、文件关联、隔离的
强制退出恢复及跨平台完整矩阵继续由 #6 跟踪。
