# #35：链接插入的字面前缀边界

跟踪：[GitHub #35](https://github.com/qinyin233/rupora/issues/35)。
修复前基线：`66ad1c92dfe0aeef5ca545f0ddbe4eb7579a0827`。

## 复现与原因

- `!x` 只选择 `x`，链接命令生成 `![x](https://)`，解析为图片；投影成为 `▧ x`。
  URL 粘贴和非图片资源链接入口也会把前面的 `!` 吸收为图片语法。
- `\x` 只选择 `x`，链接命令生成 `\[x](https://)`，开括号被转义，
  解析不出链接，原可见反斜杠消失。
- 单独的 `x` 和偶数个前导反斜杠作为对照。最小 `!x` 回归在修复前连续两次失败；
  扩展至三个编辑入口后四项测试失败。

复现命令：`cargo test --locked -j 1 --test link_boundaries -- --nocapture`。
测试通过实际编辑 API，再检查 pulldown-cmark 事件、HTML 和 VisualProjection，
排除了单纯选区偏移或显示层误判。

## 修改

三个入口共用私有前缀保护函数：在未转义的 `!` 前增加必要转义；在新标记前遇到
字面奇数反斜杠时补全转义。返回的字符选区同时后移一位，字节位置重新计算。
图片插入不转义前面的字面 `!`，仍生成图片。

已有转义中的局部源码选择保留原语法编辑行为；代码块、行内代码和原始 HTML 内
不增加前缀转义。检查解析范围时使用已存在的前一个 ASCII 字符，而不是插入位置，
避免 EOF 不属于任何半开范围造成误判。Undo 通过既有文档事务精确恢复原始源码和选区。

## 回归与审查

- 9 项集成测试覆盖三个入口、奇偶反斜杠、已转义/连续叹号、中文和 emoji、
  标签已有强调样式、空链接、图片类型、返回选区/光标以及代码和 HTML 的 EOF。
- 应用级快捷键测试在源码、写作和分屏分别核对链接字符范围，及 Undo/Redo 的
  完整源码和选区。共用实际 Ctrl+K 调度入口。
- 规范审查无违反项；需求审查发现的 EOF 代码上下文遗漏已用失败回归确认、修复，
  再审未发现其他阻断问题。

## 本地验证

`cargo fmt --all -- --check`、`cargo test --all-targets --locked -j 1 --quiet`
及 `cargo clippy --all-targets --locked -j 1 -- -D warnings` 通过。
全量测试 728 项通过、0 失败、4 项忽略；日志为 `target/issue35-all-tests.log`
和 `target/issue35-clippy.log`。`cargo build --release --locked -j 1` 通过，
耗时 4 分 21 秒，日志 `target/issue35-release.log`。
#35 在远程检查完成前保持开放。

## 原生失败复现

2026-09-30，Windows 11 家庭中文版 10.0.22631 x86_64，Computer Use 实际操作。
旧程序为 91dc441 本地 release，路径 `target/acceptance-issue34-20260930/rupora.exe`，
SHA-256 `47e3ef1ce663acdfb329fd2aa0039319024474c08ec13623a1ca57a2c5b25f18`，
版本 `2.0.0-alpha.4`，未签名。不是公开下载包。

新建合成文件 `target/acceptance-issue35-20260930/link-prefix.md`，初始 `前!甲🙂尾`。
写作模式点击正文，Home、Right 两次、Shift+Right 两次，仅选中 `甲🙂`，
Ctrl+K、Ctrl+S 后磁盘确为 `前![甲🙂](https://)尾`。Escape 失焦后出现图片占位符，
字面叹号消失，与自动回归一致。没有点击链接或访问占位地址。

Ctrl+Z、Ctrl+S 恢复原文，SHA-256
`b7f38e3b9f9743d7d678bdecae6a0e200d3d44e1d5deeaf3b65741b3398e741e`。
正常关闭并确认窗口消失。截图在本线程 Computer Use 输出，窗口 3804322，
未另存 PNG；这是普通快捷键操作，不计为输入法组合验收。
完整原生矩阵由 #6 跟踪。

## 修复版原生复测

修复提交 `2a734f228db4598cbc7acba2a6aa7e26c454dd37`，同一环境，普通本地 release，
路径 `target/acceptance-issue35-20260930/rupora.exe`，35,653,120 字节，NotSigned，
SHA-256 `8c65286b66470097ce1f4f248f1e1150a9c3f8f6b922af9e65a64376e5228ddd`。

打开上文已恢复的同一文件，在 emoji 后聚焦，Shift+Left 两次反向选中 `甲🙂`。
Ctrl+K、Ctrl+S 后源码为 `前\![甲🙂](https://)尾`，Escape 失焦后仅标签显示链接样式，
叹号和两侧中文保留，没有图片占位符。选区包含完整中文和 emoji。

Ctrl+Z、Ctrl+S 精确恢复原文及上文原始哈希；Ctrl+Y、Ctrl+S 恢复链接源码，
哈希与第一次保存链接时相同：
`fb6735efa32bec507281c33b040ce2c2479b5067c97224894e5b92ae851c66b4`。
切到源码视图，内容与磁盘一致，随后正常退出。截图在本线程工具输出，窗口 9047168。
本轮未实际测试 URL 粘贴、资源对话框或反斜杠案例的 GUI 路径；它们由上述自动回归覆盖，
不能据此宣称三模式完整原生矩阵已经通过。公开发布资产和标签未变。

修复提交的 [push CI](https://github.com/qinyin233/rupora/actions/runs/36668027952) 和
[手动 fuzz CI](https://github.com/qinyin233/rupora/actions/runs/36668049125) 已启动，
最终结果仍待检查；普通 push 的 fuzz 跳过不算通过。

## 后续输入覆盖

追加的应用级回归分别向源码、写作、分屏交付真实 egui `Paste` 事件，确认粘贴 URL
被解析为实际链接目标，而非 Ctrl+K 的占位地址；验证标签正文、链接范围、折叠光标，
以及 Undo/Redo 的完整源码和选区。

`tests/link_followup.rs` 从已创建链接的文档继续编辑，遍历包含文末在内的所有光标位置、
全部连续选区，用中文/emoji、空格和删除替换，检查重新解析后的可见正文和光标。
四组字面/转义前缀全部通过。此测试检查文字和光标，不宣称验证所有后续样式或目标 URL。
两项针对性测试、格式检查和严格 all-target Clippy 通过；本次只增加测试，
产品实现与上文已完整验证的 `2a734f2` 相同，不重复构建 release。
