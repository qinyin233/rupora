# 产品输入测试：2026-09-10

## 方法与范围

先阅读 `SOURCE-INPUT-RENDER-AUDIT-2026-09-10.md` 及现有应用层回归，避免重复上轮已修问题。本轮新增 `native/src/product_input_tests.rs`，由 `app.rs` 的测试模块挂载；不改生产代码或已有测试，不操作桌面，不提交 Git。

测试使用真实 egui `Context::run_ui`、`RawInput`、`TextEdit`，执行应用快捷键处理和源码/写作编辑器。每个场景使用隔离临时文档。字符选区是 Unicode 标量索引；实际内容直接比较 `Document.content`。这验证 headless UI 事件行为，不代表已测试真实 Windows IME 后端或鼠标操作。

运行：`cargo test --lib product_input_ -- --nocapture`。逐轮输出保存于 `target/product-input-round1.log` 至 `target/product-input-round5.log`。修复由主代理统一实施；表中“修复前”以本轮实际运行输出为证据。

## 确认的问题

### I01 — P1：跨块选区后还有未选中的第三块时，绘制 panic

- 最小复现：`one\n\ntwo\n\nthree`，选择 `1..7`，调用一次无输入事件的写作编辑器帧。
- 预期：绘制选择，文档不变。
- 修复前实际：`app.rs` 的 `hybrid_region_selection` 中减法溢出。
- 根因：`(start < end).then_some(start - region_start..end - region_start)` 会急求值；第三块位于选区末尾之后，`end - region_start` 下溢。
- 回归：`product_input_selection_before_unselected_third_block_does_not_panic`。主代理修复后已通过。
- 边界：直接观察到 debug panic；没有将此冒充已验证的 release 崩溃。

### I02 — P1：Ctrl+A 后同帧输入使用旧选区或删除块分隔符

- 文档：`正文🙂\n\n- [ ] KEEP`，光标 `1..1`；同帧 `[Ctrl+A, Text("替换")]`。
- 写作模式预期：`替换\n\n- [ ] KEEP`，与两帧操作一致。
- 修复前实际：`替换- [ ] KEEP`，任务列表被接到段落后。
- 两种模式同帧 `[Ctrl+A, Text("(")]` 还会得到 `正()文🙂\n\n- [ ] KEEP`；分帧操作会包裹已选内容。
- 根因：`editor_input_action` 的批量判断未统计 Ctrl+A；智能配对重放使用输入前旧选区；写作编辑器对块局部全选的修正又发生在 TextEdit 已消费输入之后。
- 回归：`product_input_select_all_same_frame`，6 个组合（两模式×普通文字/圆括号/方括号），与分帧行为比较。主代理最终修复后 round4 已通过。

### I03 — P1：跨块选择后按方向键，后续输入仍删除原选区

- 文档 `one\n\ntwo`，选区 `1..7`。
- ArrowLeft 再输入 `X` 预期 `oXne\n\ntwo`；ArrowRight 再输入预期 `one\n\ntwXo`。
- 修复前实际：两者均 `oXo`，原跨块内容被删除。分帧和同帧都实际复现，按键和帧修饰键均为 NONE。
- 根因：跨块选择独立于局部 TextEdit；导航未折叠全局选区；`take_cross_block_input` 还会越过导航事件优先消费后面的文本。
- 回归：`product_input_cross_block_navigation_before_text`。

### I04 — P1：IME 同帧提交后启动下一组合，取消时丢失已经提交的文字

- 初始 `AB`、光标 1。
- 帧1 `Preedit("ni")`；帧2 `[Commit("你"), Preedit("hao")]`；帧3 `Commit("")` 取消第二次组合。
- 预期 `A你B`；写作模式实际 `AB`。把帧2拆成两帧，及源码模式，都正确保留 `你`。
- 根因：`ime_frame_action` 只保留帧内最后的 Preedit 状态，整个帧被当作未提交组合延迟；后续取消丢弃包括已经提交文字的会话。
- 回归：`product_input_ime_committed_text_survives_next_composition_cancel`。

### I05 — P2：Ctrl+Home/End 只在当前块导航，Shift 选择同样受限

- 文档 `first\n\nmiddle\n\nlast`，光标 9，写作模式。
- Ctrl+Home 再输入 `X` 预期文档首插入；实际 `first\n\nXmiddle\n\nlast`。
- Ctrl+End 再输入预期文档末追加；实际 `first\n\nmiddle\n\nXlast`。
- Ctrl+Shift+Home 预期替换全文首到光标的范围，实际只替换当前块前部；Ctrl+Shift+End 同样未选到全文末。
- 根因：文档导航交给块局部 TextEdit，使用块内范围。
- 回归：`product_input_control_home_end_navigate_whole_document`，四个组合。桌面同类现象由另一测试员独立检查，本报告证据为 headless 运行。

### I06 — P2：围栏边界导航和输入同帧时，文字进入错误块

- `前文\n\x60\x60\x60rust\ncode\n\x60\x60\x60\n\nTAIL`，前文末 `[Delete, Text("X")]`：分帧进入代码正文并在 `code` 前插入；同帧仍写在 `前文` 后。
- 代码正文起点 `[Backspace, Text("X")]`：分帧回前段；同帧在代码正文前插入。
- 代码正文末 `[ArrowRight, Text("X")]`：分帧在下一段 `TAIL` 前插入；同帧追加在 `code` 后。
- 根因：`editor_input_action` 的 `single_action` 屏蔽整个批量帧的边界导航钩子，原生局部 TextEdit 无法代替文档跨块导航。
- 回归：`product_input_fence_boundary_multi_event_matches_sequential`，四个边界组合，三个失败。围栏用 `\x60` 表示反引号，可运行测试保留原始 Markdown。

### I07 — P2：同帧较晚发生的 Undo 抢先执行

- 两种编辑器初始 `AB`，先输入 `C` 得 `ABC`；下一帧 `[Text("X"), Ctrl+Z]`。
- 同事件顺序分帧得到 `AB`（连续键入合并撤销）；同帧实际 `ABX`。
- 根因：`handle_shortcuts` 在编辑器消费文本前扫描全帧并执行撤销，使较晚的 Ctrl+Z 越过较早的 Text。
- 回归：`product_input_text_then_undo_in_same_frame_obeys_event_order`。

### I08 — P1：删除代码后的段落再输入，破坏闭围栏

- 根据桌面测试员提供的场景补独立 headless 回归。源文是 Rust 围栏代码 `code`、空行、`para KEEP`、空行、`END`。
- 写作模式在 `para KEEP` 中定位，分帧 Ctrl+A、Delete、Text("REPLACE")。
- 预期保留代码后段落：闭围栏后 `\n\nREPLACE\n\nEND`。
- 实际闭围栏与 `REPLACE` 紧贴同一行，之后积累四个换行：`\x60\x60\x60REPLACE\n\n\n\nEND`。直接 Ctrl+A 后输入、不单独 Delete 的对照通过。
- 根因线索：删除段落后 `pending_edit` 重算活动块；`block_for_char_index` 把块间空白分配给之前代码块，`hybrid_edit_range` 却又排除代码块后空白，下一次输入位置落到闭围栏边缘。
- 回归：`product_input_delete_paragraph_after_fence_then_type_preserves_boundary`。

### I09 — P2：源码末尾切写作模式，当前光标所在块未进入视野

- 根据桌面反馈补回归：90 个普通段落后追加 `FINAL_TARGET`，源码模式将光标置于全文末，再切写作模式。
- 连续五个真实绘制帧检查目标文字的包围盒是否与 `clip_rect` 相交。
- 预期至少有一帧可见；实际全部不可见，文档内容不变。
- 根因线索：SetView 保留了待恢复光标，但写作模式没有和阅读模式类似的目标块滚动处理；文本位置恢复不等于滚动可见。
- 回归：`product_input_switch_from_source_tail_to_hybrid_keeps_caret_visible`。

## 通过场景及状态

- 跨中文/emoji/围栏范围 Delete、Backspace、Cut，随后键盘 Ctrl+Z / Ctrl+Y 精确恢复和重做。
- 两模式 IME Commit 后同帧继续普通文字输入。
- 写作模式 Unicode 选区替换后切源码、撤销、再切回写作重做。
- 两模式代理对字符、补充平面汉字、组合附加符、ZWJ 家族 emoji、Tab、CRLF/CR 粘贴，删除后撤销/重做；检查粘贴原文及 LF 约定。

round5：13 个测试，6 通过、7 失败。随后主代理修复后的 `target/product-root-input.log` 显示 **13 个测试，10 通过、3 失败**：I01、I02、I03、I05、I06、I08 已验证通过；剩余 I04（IME 批量取消）、I07（Undo 事件顺序）、I09（切模式目标可见）由主代理继续修复。

此结果是中途状态，不代表最终交付结果。专属测试文件已 `rustfmt --edition 2024`。无全量构建、Clippy、release 或桌面测试结论。
