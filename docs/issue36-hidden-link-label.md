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

本节记录初始修复提交 `867962d`，Windows 本地 Rust `1.93.1`；后续性能修复单列于文末：

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

该提交的精确源码 CI/fuzz 最终结果见文末。普通 push 的 fuzz 跳过不算通过。

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
这与 10 月 1 日旧版复现记录的系统版本不同。程序构建时对应本次未提交修复，
后续提交为 `867962d87dc180df8667eedb2e70f0fbacfcc02d`；
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

上述可执行文件复测使用实际原生窗口、普通快捷键及字面 Unicode 输入，不计为系统 IME 组合验收；
运行对象是本地编译程序，没有执行安装包。系统组合输入、安装、文件关联、隔离的
强制退出恢复及跨平台完整矩阵继续由 #6 跟踪。

## 当前本地包与实际系统拼音

同一修复提交以 cargo-packager `0.11.8` 构建本地 NSIS 候选：
`target/acceptance-issue36-20261003/packages/rupora_2.0.0-alpha.4_x64-setup.exe`。
包为 13,229,645 字节、`NotSigned`，SHA-256：
`9e6413765e8db7505dd3aeb898ad1f230a5c8c4cf3f6a0376e4fe49bd4f1d7db`。
既有 7-Zip `22.01` 检查全部 8 个条目成功，并解包到同目录的 `extracted/`。
包内程序完整 SHA-256 与上节实际验收程序一致：`33e34f49…bf9d82`。
没有运行安装器。机器可读记录为该目录的 `payload-verification.json`。

实际从 `extracted/rupora.exe` 启动并操作，Windows 版本沿用本轮实际查询的
`10.0.26300` x86_64。微软拼音组件版本 `10.0.26100.9278`；注册表 AppliedDPI
为 120（125% 配置，不是逐显示器实测）。逐键操作确实显示了系统拼音候选框，
与上节字面 `type_text` 输入分开记录：

1. 初始磁盘为上节最终结果，SHA-256 `0cdc1b6d…f2d6ed`。写作模式点击 `中` 与
   emoji 之间，逐键 `x`、`i`、`n`，候选第一项为“新”。候选未确认时按 Ctrl+S，
   候选仍显示，磁盘哈希不变；这不证明无改动保存一定执行。
2. 空格确认、Ctrl+S 后首行为 `前[&#93;中新🙂b][a🙂b]尾`，无残留拼音或重复字。
   文件 SHA-256 为 `dd1ff5ed49b8c8d08741d76001512c96e3bcdb312142c869f4113bcc54ec7604`。
   一次撤销保存恢复 `0cdc1b6d…f2d6ed`，一次重做保存恢复 `dd1ff5ed…ec7604`。
3. 逐键 `z` 显示候选，Escape 取消，再保存，文件仍为 `dd1ff5ed…ec7604`。
   随后逐键 `m`、`o`，第一候选为“末”；空格确认保存后首行为
   `前[&#93;中新末🙂b][a🙂b]尾`，引用定义未变。文件为 UTF-8 无 BOM、LF、
   有末尾换行，共 67 字节，SHA-256：
   `0f400e716cfa14ca79ac9698f533a85262a206c27f8b4f50248e0ba415de5df7`。
4. Escape 结束源码显露，可见文本为 `前]中新末🙂b尾`，链接和相邻文字样式正确。
   正常退出、确认进程消失，从同一解包位置重启，路径、正文渲染及完整文件哈希均保持。
   随后正常关闭。窗口为 `1313904`、`2297450`；截图在本线程工具输出。

这补充了当前包内程序的实际拼音确认、取消、下一次确认、单次历史与正常保存重启。
完整三模式输入矩阵、第二种输入法、安装与文件关联、隔离的强制退出恢复及其他平台
仍未通过；#6 保持开放。公开 Release 资产和标签没有改变。

## 远程验证

修复提交 `867962d87dc180df8667eedb2e70f0fbacfcc02d` 的
[push CI 37042002015](https://github.com/qinyin233/rupora/actions/runs/37042002015)
已完成且成功：Windows、Ubuntu、macOS 测试、严格 Clippy、release 构建，以及安全和
性能门禁均通过。该运行的 fuzz 按配置跳过，不算 fuzz 成功。

同一源码的 [手动 CI 37042088030](https://github.com/qinyin233/rupora/actions/runs/37042088030)
已完成且成功，包含三平台、安全、性能和实际 fuzz。已读取 job `110954419073`
的解码日志，三个目标的 `-max_total_time=60` 实际执行结果如下：

| 目标 | 实际运行次数 | 实际时间 |
| --- | ---: | ---: |
| `markdown_pipeline` | 62,069 | 61 秒 |
| `table_parser` | 1,121,152 | 61 秒 |
| `wysiwyg_projection` | 76,678 | 61 秒 |

这证明该版本的有限 fuzz 运行通过，不代表后续源码已经通过，也不证明没有未知问题。

## 大输入性能复审

初始修复的保护编码对每个字符重新扫描全部完整行内范围，嵌套图片等范围也在该集合中。
在真实 `VisualProjection::apply_edit` 接口，用完整 `` `[` `` 片段后接一个未匹配 `[`
替换 `[a](u)` 的隐藏标签，确认文字和链接样式仍正确，但耗时呈平方增长：
2k、4k、8k、16k、32k 片段分别约 0.004、0.016、0.055、0.228、0.862 秒；
128k 片段（512,001 字节）为 13.338 秒。

将该案例加入实际 `perf_guard` 后，修复前在 release 模式明确失败：13.471 秒，
超过新增的 2 秒预算，退出码 101。日志 `target/issue36-perf-red.log`；
[追踪记录与验收补充](https://github.com/qinyin233/rupora/issues/36#issuecomment-5958290371)。
这是测量确认的性能退化，不是 CI fuzz 崩溃。

后续修复按起点排序完整行内范围，字符遍历时只向前推进范围游标，保留重叠范围的并集
语义，避免每字符重新扫描。新增图片内代码/Math、HTML 与中文/emoji 混合输入回归，
核对文字、保留样式、原链接目标和光标；大粘贴性能案例继续留在常规门禁中。
后续修复在 Windows Rust `1.93.1` 的本地检查：

- `cargo fmt --all -- --check` 通过。
- `cargo test --all-targets --locked -j 1 --quiet` 通过：741 项通过、0 失败、4 项忽略；
  `target/issue36-followup-all-tests.log`。包含 12 项隐藏标签集成回归。
- `cargo clippy --all-targets --all-features --locked -j 1 -- -D warnings` 通过；
  `target/issue36-followup-clippy.log`。
- `cargo run --release --locked -j 1 --example perf_guard` 通过，退出码 0；
  `target/issue36-followup-perf.log`。同一 128k 片段输入从修复前 13.471 秒降为
  0.120 秒，文字及链接样式断言同时通过；全部 10 项预算通过。
- `cargo build --release --locked -j 1` 通过，退出码 0，5 分钟；
  `target/issue36-followup-release.log`。

性能修复提交 `a63d8e0ae5abd5bfd7f332227a19f9a5ec4546e6` 的
[push CI 37048199131](https://github.com/qinyin233/rupora/actions/runs/37048199131)
和 [手动 CI 37048283680](https://github.com/qinyin233/rupora/actions/runs/37048283680)
均已完成且成功，三平台测试、严格 Clippy、release、安全和性能门禁通过。
push 的 fuzz 跳过；手动运行的实际 fuzz job `110975037655` 成功，解码日志确认：
`markdown_pipeline` 56,448 次、`table_parser` 948,012 次、
`wysiwyg_projection` 63,430 次，分别实际运行 61 秒。
这些有限运行只验证上述精确源码，不证明没有未知缺陷。

## 性能修复版原生回归

2026-10-03，另存最新 release 程序到
`target/acceptance-issue36-20261003/optimized/rupora.exe`，35,677,696 字节，
Authenticode `NotSigned`，SHA-256：
`2000c49c24fb2e6de2323e33c6da0f9b4595ebe6b3e15b5624ff9686f4e22356`。
环境仍为本轮实际查询的 Windows `10.0.26300` x86_64。
同目录新夹具从原始 `前[a🙂b][]尾` 及原引用定义开始，初始哈希为 `ee897762…eaef7`。

真实写作窗口仅选中 `a`，输入字面 `]`，保存结果为 `前[&#93;🙂b][a🙂b]尾`，
完整哈希 `e5837703…e11cc`。一次撤销、保存恢复原始完整哈希；一次重做、保存恢复
该修复哈希。在返回光标处继续输入字面 `中` 后，首行为 `前[&#93;中🙂b][a🙂b]尾`，
完整文件 SHA-256 为 `0cdc1b6d60081ab0609a04f10e7a146e75b67488784faa128a3a648d8af2d6ed`。
Escape 结束源码显露，写作与阅读画面均显示 `前]中🙂b尾`，保留链接和相邻文字样式。

正常退出、确认进程已消失，从同一程序路径重启，恢复了该夹具的路径、正文和完整
文件哈希；再次 Escape 后画面保持正确。窗口为 `4915762`、`462426`，截图在本线程
工具输出。随后正常关闭并确认无残留进程。
这是性能修复版的普通原生输入与保存重启回归；上节实际拼音与解包验收仍只对应
`867962d` 程序，不据此将完整输入法或安装矩阵记为通过。

## Standards

独立规范复审未发现违反仓库记录的职责边界或 Unicode 坐标规则。两个可选的重复代码
整理项为：共享空格实体及其长度；使用已有样式范围函数比较未修改的前后缀。
它们是维护性判断，不是本次性能修复的阻断项。范围排序与游标改动没有新增规范问题。

## Spec

独立需求复审未发现已确认的实现缺陷或范围扩张。按起点排序后，当前范围未结束时
覆盖当前位置；结束后跳过已失效的重叠范围，再检查下一个范围，保持原来的范围并集语义。
转义对跳过仍保留单调的 UTF-8 字节遍历和逐字符边界表。复审时待完成的本地门禁
已补齐于上节；随后也确认了精确源码 CI/fuzz 成功，前一提交的结果未被用于代替这些验证。

两轴摘要：Standards 0 项硬违反、2 项可选重复整理；Spec 0 项确认缺陷，精确源码验证已完成。
