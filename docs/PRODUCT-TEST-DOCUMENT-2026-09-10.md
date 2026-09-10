# RUPORA 文档产品测试（2026-09-10）

## 范围与边界

本轮以产品行为为验收条件，独立测试跨标签选择与编辑历史、保存、磁盘外部修改、恢复合并、可视化表格，以及引用链接/图片/脚注。先读取 `SOURCE-INPUT-RENDER-AUDIT-2026-09-10.md`，未将其已经修复的 19 项重复算作新缺陷。

测试使用真实 `RuporaApp`、`Document`、`RecoveryStore`、Markdown 解析器和 `egui::Context::run_ui`；两种画布检查实际绘制输出。所有文件、恢复快照和 PNG 均在 `tempfile::tempdir()` 下创建。没有操作真实桌面、用户文稿或恢复目录；没有修改生产代码和既有测试。新增测试集中于 `native/src/product_document_tests.rs`，由主代理在 `app.rs` 挂载。修复由主代理执行。

## 已实际复现的问题

### D1 — P1：跨块选区遇到后续未相交段落时下溢

最小内容：`甲乙\n\n丙丁\n\n末尾`。写作模式设字符选区 `1..7`，渲染一帧；此时前两段被选中，第三段完全在选区之后。

预期：只绘制相交部分的选区。实际：调试测试在 `hybrid_region_selection` 计算 `end - region_start` 时 panic，`attempt to subtract with overflow`。

根因：`(start < end).then_some(...)` 会提前计算参数，即使范围不相交。修复建议：仅在范围相交时计算局部偏移。主代理改为惰性 `then(|| ...)` 后，同一测试通过；其后跨标签选区恢复、输入、撤销、重做、保存以及第二标签隔离检查也通过。

证据测试：`product_document_tab_selection_undo_redo_and_save_are_isolated`。

### D2 — P1：表格输入奇数个反斜杠加管道，后续列内容丢失

最小操作：在两列表格第一行第一列输入字面字符 `x\|y`，第二列输入 `KEEP`，应用表格。

预期：字段按原生行内 Markdown 解释，第一列语义为 `x|y`，第二列保留 `KEEP`。允许源码转义规范化，不要求输入字段逐字相同。

实际：序列化无条件在管道前增加反斜杠，输出的管道前变为偶数反斜杠，管道被解析成新列边界。重新解析后第一、二列变成反斜杠文本和 `y`，原有 `KEEP` 被截掉。

扩展矩阵：管道前 1、3、5 个连续反斜杠均复现；0、2、4 个通过。单元格末尾 0 到 5 个反斜杠均通过。另覆盖中文、组合字符、emoji、代码片段中的管道和 Markdown 转义。

根因：`escaped_cell` 没有结合管道前反斜杠奇偶性执行 GFM 管道转义；之后 `normalize` 将意外产生的多余单元格截掉。建议按行内 Markdown 的语义保持转义，验证重新解析后的列位置和首列 HTML 语义。

证据测试：`product_document_table_new_cell_pipe_backslash_roundtrip`；失败日志 `target/product-document-round2.log`。主代理修复后，第三轮该扩展矩阵通过。

### D3 — P2：引用式链接与引用式图片在原生画布丢失解析上下文

链接最小内容：

```markdown
See [Documentation][docs].

Other paragraph.

[docs]: https://example.com/docs "Docs title"
```

预期：两种画布显示可识别的 `Documentation` 链接。实际：原生画布绘制原串 `[Documentation][docs]`；同一全文交给 HTML 渲染器时正确生成链接。

图片使用真实的本地 1×1 PNG，内容如下：

```markdown
![Diagram][img]

Some paragraph.

[img]: pixel.png
```

预期：显示图片及其说明。实际：阅读模式和写作模式都绘制 `![Diagram][img]` 原串。同一全文 HTML 正确生成 `<img>`。该失败不是网络或图片文件不存在造成的。

根因：原生画布把 Markdown 切成块后独立解析，各块没有全文末尾的引用定义。修复需共同覆盖静态预览、图片识别、点击目标以及活动块投影，并保持所有输入/指针位置仍映射到原文范围。

证据测试：`product_document_reference_links_render_in_both_reading_canvases`、`product_document_reference_image_renders_in_both_reading_canvases`；失败日志 `target/product-document-round2.log`。另新增活动块输入验证 `product_document_reference_link_stays_resolved_while_editing_surrounding_text`。

同源排查：跨块脚注的标记和正文在两画布均正确绘制；本轮未将脚注归为该缺陷，也没有声称已验证脚注点击导航。

## 通过的产品流程

- 两种编辑器：跨标签保存各自选区，跨块替换后撤销/重做恢复准确，保存后的磁盘内容与内存一致，另一标签未受修改。D1 修复后通过。
- 外部修改：干净文件自动加载新内容；有本地修改的文件保持本地文本、登记冲突，未经覆盖授权的 `save(false)` 拒绝写盘，外部版本不变。
- 恢复快照：独立的本地/外部变更自动合并；冲突时同时保留 LOCAL 和 EXTERNAL；恢复本身不写盘，显式保存后磁盘与内存一致。
- 表格删除：反复删除最后一行/列仍保留合法的一列表头；前后段落、保存结果和撤销内容一致。
- 脚注：跨块引用与脚注正文在两种画布均可见，未泄露原始 `[^note]` 标记。

## 运行记录

命令：`cargo test --locked product_document_ -- --nocapture`。

首轮 6 项：3 通过，3 失败（D1、D2、D3链接）。D1 修复后定向选区测试通过。

第二轮 8 项：5 通过，3 失败（D2、D3链接、D3图片）。日志：`target/product-document-round2.log`。

第三轮 9 项：6 通过，3 失败。D2 表格修复通过；D3 的链接、图片、活动块引用测试仍失败，活动块也实际绘制了原串。日志：`target/product-document-round3.log`。由主代理后续修复与全量门禁的结果判定最终交付状态。本报告记录本轮真实复现，不将定向测试等同于真实桌面交互或三平台验证。
