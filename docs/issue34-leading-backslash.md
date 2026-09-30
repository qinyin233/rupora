# #34：反斜杠后的格式命令

跟踪：[GitHub #34](https://github.com/qinyin233/rupora/issues/34)。
复现基线：`dc5d21c5c556632123f8c012f06d78161893b98d`。

## 复现与原因

源码为 `\x`，只选择 `x`（字符范围 `1..2`），加粗或斜体未生效，源码保持原样。
以下回归命令在修复前两次失败；无反斜杠和两个源码反斜杠的对照均通过：

```text
cargo test --locked -j 1 --test inline_html_emphasis emphasis_after_a_literal_backslash_preserves_visible_neighbors -- --nocapture
```

前一个字面反斜杠会转义新加入的 Markdown 标记或 HTML 开标签。
#33 已处理选区末尾的反斜杠，但遗漏了选区前的这个边界。
修复只在 HTML 回退需要时将此前缀写为两个源码反斜杠，仍显示一个未加格式的反斜杠，
格式包装与返回选区向后移动一字节。已有 ASCII 转义内部的源码选区不走此修复。
仍由解析器验证包装有效后才提交修改。

## 表示与历史

取消样式后可以保留上述等价转义，例如 `\x` 最终为 `\\x`；可见正文必须相同。
该新增转义仅用于选区边界，不能改写其他邻居。Undo 精确恢复命令前的源码及选区，
Redo 精确恢复命令结果。HTML 导出、投影正文和逐字符样式共同验证这一约定。

## 回归覆盖

- 1、2、3、4 个前导反斜杠，中文与 emoji 邻居，选区两端同时需要转义。
- 加粗/斜体两种顺序，移除其中一种、全部移除，再加上与移除，核对每个字符样式和选区。
- 行内代码及已有 ASCII 转义内的局部选择不被前缀修复改写。
- 源码、写作和分屏快捷键：组合格式、逐次 Undo/Redo，核对完整源码及选择范围。
- 属性测试只允许在满足条件的两个边界各增加至多一个反斜杠，并要求 HTML 输出相同。
  其他源码往返差异仍判失败。

## 审查和验收范围

### 需求审查

0 项阻断问题；建议的取消后重加用例已补充。完整本地检查与 CI 是关闭前的验证条件。

### 规范审查

0 项规范违反；1 项可维护性建议已处理：反斜杠奇偶计数重复已提取为私有函数并再次审查。

### 本地验证（2026-09-30）

- `cargo fmt --all -- --check` 通过。
- `cargo test --all-targets --locked -j 1 --quiet`：718 项通过、0 失败、4 项忽略；
  忽略项不记为通过。日志：`target/issue34-all-tests.log`。
- `cargo clippy --all-targets --locked -j 1 -- -D warnings` 通过；日志：`target/issue34-clippy.log`。
- `cargo build --release --locked -j 1` 通过，耗时 4 分 02 秒；日志：`target/issue34-release.log`。
- 提交后的 GitHub Actions 结果在 #34 跟踪，CI 尚未验完时保持问题开放。

这些是源码及自动化验证。本问题尚没有新构建的真实桌面操作结果；完整原生矩阵仍由 #6 跟踪。
公开 Release 和标签未改动。
