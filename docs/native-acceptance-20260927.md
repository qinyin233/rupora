# Windows 原生验收：2026-09-27

对应 [#6](https://github.com/qinyin233/rupora/issues/6)。本轮由 Codex 通过
Computer Use 操作实际 NSIS 解包程序和微软拼音，记录写作模式的 I1–I4，
以及提交后切换源码、分栏的部分 I6。其余项目仍待验收。

## 环境与资产

- Windows 11 家庭中文版 x86_64，10.0.22631。
- `HKCU/Control Panel/Desktop/WindowMetrics/AppliedDPI` 为 120（125%）；
  这是系统配置读数，未另测每个显示器的有效 DPI。
- 微软拼音，中文拼音逐键输入；ChsIME.exe 为
  `10.0.22621.5697 (WinBuild.160101.0800)`。实际观察到系统候选框；
  未验证其他输入法、双拼或其他键盘布局。
- 候选源码：`1696b07e499858841629a4d55033d4f150100e72`。
- 本地 NSIS：`target/acceptance-1696b07/packages/rupora_2.0.0-alpha.4_x64-setup.exe`，
  13,216,745 字节，SHA-256：
  `f1e1691faa7d3361ede2a30b79855ad949e32543624cc0acde999a4ab00dfdd5`。
- cargo-packager 0.11.8、Rust/cargo 1.93.1；已有 7-Zip 校验并解包。
  实际运行 `target/acceptance-1696b07/extracted/rupora.exe`，35,621,376 字节；
  本轮重新读取 SHA-256 为
  `928bac964251742b433f255d60d2d5d272d2aca5fd0497e0f4f106143e56392e`。
- 显示版本为 2.0.0-alpha.4，Authenticode 未签名；这是本地开发候选，
  无公开下载地址或对应来源证明，不是旧的公开 alpha.4 下载资产。
  未执行安装器，未修改公开 Release 或标签。

## 操作与结果

使用合成文件 `target/acceptance-1696b07/run-20260927/ime-baseline.md`，
初始 UTF-8 无 BOM 正文为 `A🙂B`，无尾随换行。通过系统打开对话框加载，
以空撤销历史开始。所有拼音均由独立按键输入，未使用直接 Unicode 输入代替 IME。
原生截图和逐步可访问文本保留在 Codex 本任务工具记录，未另外导出图片。

| 用例 | 操作 | 实际结果 |
| --- | --- | --- |
| I1，写作 | Home、Right 定位 A 后；逐键 zhongwen，观察候选“中文”，空格确认 | `A中文🙂B`，无拼音残留；候选框位于编辑行下方 |
| I4，写作 | 对上述提交按一次 Ctrl+Z，再一次 Ctrl+Y | 分别为 `A🙂B`、`A中文🙂B`，未产生多个预编辑历史项 |
| I3，写作 | 从重做后的光标 Right 越过 emoji，两次 Shift+Left；可访问选区确认为 `文🙂`；逐键 xin，确认“新” | `A中新B`，emoji 完整删除，前后正文保留 |
| I3 撤销 | 对替换按一次 Ctrl+Z | `A中文🙂B`，选区恢复为 `文🙂` |
| I2，写作 | 再撤销一次回到 `A🙂B`；A 后按 z，观察候选后 Escape；随后逐键 mo，确认“末” | 取消后 `A🙂B`，再次提交后 `A末🙂B`，无幽灵预编辑 |
| I6 部分 | 提交“末”后依次点击源码、分栏 | 源码为 `A末🙂B`，分栏源码和右侧预览一致 |

I2 使用撤销到初始基线的同一文件执行，未另开独立新文档；以上是实际操作顺序。
三次 Ctrl+S 后均直接读取磁盘，结果如下（十六进制为完整文件字节）：

| 保存时正文 | UTF-8 字节 |
| --- | --- |
| `A中文🙂B` | `41E4B8ADE69687F09F998242` |
| `A中新B` | `41E4B8ADE696B042` |
| `A末🙂B` | `41E69CABF09F998242` |

最终文件 9 字节、无 BOM/尾随换行，SHA-256 为
`33b67b5145f5477b348eb1a6c80b8f9be2945d44e931d8fde2f164ff35a48e6a`。

## 边界与后续

此前把浏览器工具的原生控制限制误认为所有 Computer Use 都不可用；本轮通过独立
`@oai/sky` 接口完成上述操作，旧的“无法原生控制/解包程序未启动”结论已失效。

本轮未发现新缺陷，未修改产品源码。实际验收使用当前账户的合成文件，应用还恢复了
以往合成测试标签；不是独立测试账户或虚拟机，因此没有进行强制终止或恢复测试。
源码/分栏模式的输入矩阵、I5、完整 I6、I7、I8、文件关联、当前包的退出重启与
P3/P4 恢复仍待完成。其余平台和第二种输入法未验证，不能以旧包结果替代。

候选的 [push CI](https://github.com/qinyin233/rupora/actions/runs/36300097976)
和 [手动 CI](https://github.com/qinyin233/rupora/actions/runs/36300113322) 均成功。
push 的 fuzz 跳过；手动运行的三个 fuzz 目标实际执行并成功。这不等于完整产品验收，
#6 保持开放。
