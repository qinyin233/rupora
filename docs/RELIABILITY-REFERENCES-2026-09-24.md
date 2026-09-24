# 编辑器可靠性实践对照研究

日期：2026-09-24。状态：**研究证据与实施建议，不是验收报告，也不是“已接近零 bug”的证明。**

## 1. 结论与证据边界

RUPORA 最值得补强的是可重放的**完整编辑序列、事件批次和后台结果时序**，以及原生输入法、实际安装包的验证。已有的三平台测试、Unicode 属性测试、固定种子编辑历史测试、原子保存和发布校验应继续保留；不能把它们重复列为尚未建设的能力。

本研究对照 Zed、Lapce、egui 三个项目。选择依据是可直接检查的 Rust 编辑、输入与测试实现，不使用 stars、项目规模或“成熟项目没有 bug”作为论据。Lapce 部分另查了其 GUI 框架 Floem 的官方文档，明确区分框架能力与 Lapce 实际集成。

本仓库以 `214439180f70785711c10576afcf05114493f07c` 为比较基线；主线程同时修改工作区，因此本报告不评价基线之后的修复。阅读了 `docs/QUALITY.md`、CI/发布工作流、相关产品测试、属性测试、fuzz 目标、文档保存与恢复代码。本轮未运行 Cargo、上游测试、安装包或真实操作系统输入法。

实时读取的一手源码固定如下。官方生成文档没有对应提交信息时单独标出，不把它当作已锁定的依赖版本。

| 项目 | 源码提交 | 主要用途 |
| --- | --- | --- |
| Zed | `4c902c9db22a82f5f3a14c02442e7f60ec40d9c8` | IME/编辑时序回归、受控调度、发布关卡 |
| Lapce | `b604d57de4a820006d335a3be0d7583eb8fab558` | 保存版本契约、避免照搬不同数据安全方案 |
| egui | `a55f23a1ec6e05b55a3398308009e53dabf22b90` | 输入法视觉回归、AccessKit 测试、局部截图策略 |

以下“事实”描述已读代码；“差距”限定于本次检查的基线；“建议”需要另行设计、实现并验证。

## 2. Zed：验证编辑过程及其时序

### 2.1 输入法不只检查最终字符串

**事实。** `test_ime_composition` 通过真实 Editor 的输入处理接口反复更新 marked text，检查正文及 UTF-16 marked range；还覆盖提交、组合期间 Undo、越界替换范围裁剪和多个选区。测试把自动历史合并间隔设为零，避免时间合并掩盖组合事务行为。这是编辑器内部接口测试，不能推出真实系统输入法端到端已覆盖。[Zed IME 回归源码](https://github.com/zed-industries/zed/blob/4c902c9db22a82f5f3a14c02442e7f60ec40d9c8/crates/editor/src/editor_tests.rs#L530)

**适合复用。** 给 RUPORA 的事件回放增加阶段断言：预编辑范围、带方向的选区、当前块/文档归属、提交后 Undo 边界，以及取消后下一次输入。继续走 App/EditorSurface/egui 的实际路径，不能用测试专用文本替换器代替生产输入行为。Zed 的 UTF-16 范围契约属于它的接口；RUPORA 必须继续明确区分 egui 字符索引与 Markdown UTF-8 字节范围。

### 2.2 使异步顺序可复现

**事实。** GPUI executor 提供受控时钟、单步执行和 `run_until_parked`；测试随机延迟由 `SEED` 控制。`advance_clock` 只使定时器就绪，不隐含执行任务。这样的接口能把“等待多久”和“先运行哪个任务”变成测试输入。[GPUI executor](https://github.com/zed-industries/zed/blob/4c902c9db22a82f5f3a14c02442e7f60ec40d9c8/crates/gpui/src/executor.rs#L171)

**事实。** Zed 的输入格式化回归使用 `iterations = 20, seeds(31)`，主动令自动缩进异步执行，模拟回车和 `.`，在格式化请求及最终编辑状态中检查顺序。这验证的是特定时序性质，不只是“最后没崩溃”。[输入格式化与自动缩进回归](https://github.com/zed-industries/zed/blob/4c902c9db22a82f5f3a14c02442e7f60ec40d9c8/crates/editor/src/editor_tests.rs#L28456)

**适合复用，但无需移植 GPUI。** RUPORA 可以先在现有后台结果注入入口建立小型测试驱动：指定“编辑、保存为、切换文档、收到旧结果”的顺序，明确推进模拟时间，记录种子和完整动作。现有 `BackgroundServices` 已有任务函数注入边界；应优先扩展这种边界，而非引入一套通用调度框架。[RUPORA 后台服务基线](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/native/src/background.rs)

### 2.3 发布关卡是依赖图的一部分

**事实。** Zed 的 release 工作流让平台打包依赖测试、Clippy 和脚本检查；上传后按预期文件清单检查缺件，预览版自动发布依赖资产验证与合规检查。macOS 打包脚本有条件地执行签名、公证等待与 stapling。这里只确认源码中的流程，未核查某个已发布安装包的执行结果。[Zed 发布工作流](https://github.com/zed-industries/zed/blob/4c902c9db22a82f5f3a14c02442e7f60ec40d9c8/.github/workflows/release.yml#L803)、[macOS 打包脚本](https://github.com/zed-industries/zed/blob/4c902c9db22a82f5f3a14c02442e7f60ec40d9c8/script/bundle-mac#L273)

**对 RUPORA 的意义。** 保留现有“完整性验证后才公开”的发布顺序。上游这些步骤没有证明安装后的输入、保存和恢复正确，不能代替下面提出的安装包验收。

## 3. egui：语义断言与局部视觉断言配合

### 3.1 使用当前 GUI 路径进行输入法视觉回归

**事实。** `egui_kittest` 的 `test_ime_composition_visuals` 创建多行 TextEdit、注入 `ImeEvent::Preedit`，分别对非空活动片段和折叠到光标的范围截图。这个例子能验证组合文字的视觉状态，但注入 egui 事件仍不覆盖 winit 与真实系统输入法之间的转换。[egui IME 视觉回归](https://github.com/emilk/egui/blob/a55f23a1ec6e05b55a3398308009e53dabf22b90/crates/egui_kittest/tests/tests.rs#L223)

**适合复用。** RUPORA 可以沿用当前真实 egui 帧测试，在格式标记隐藏、行内代码、跨行中文组合、窄宽换行等场景同时检查源码、caret 几何和活动范围绘制。先增加确定性的布局断言，再挑少量容易出现视觉错误的状态加入图片基线。

### 3.2 截图不能淹没语义测试

**事实。** `egui_kittest` 通过 AccessKit 按角色/标签操作控件；图片快照需要 `wgpu` 与 `snapshot` 特性。其 README 建议优先普通断言，保持快照小而聚焦，谨慎设置差异阈值，并提醒图形后端/编译特性会造成差异。启用 `UPDATE_SNAPSHOTS` 会更新基线并令测试通过，不能把该模式的成功当作回归验证。[egui_kittest 使用说明](https://github.com/emilk/egui/blob/a55f23a1ec6e05b55a3398308009e53dabf22b90/crates/egui_kittest/README.md)

**适配限制。** RUPORA 基线使用 eframe `0.35.0` 和 `glow`。当前上游 HEAD 的 kittest 示例不是已经验证兼容的依赖组合；若采用 wgpu 快照，它首先证明的是该测试渲染路径，不能冒充发布包的 glow 验收。可以先保留现有 shape/galley 断言，再评估固定字体、尺寸、缩放比例、主题与后端的局部截图方案。[RUPORA GUI 依赖](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/Cargo.toml#L49)

## 4. Lapce：版本契约值得借鉴，保存算法不能照搬

**事实。** Lapce 的官方架构说明 GUI 与 proxy 保存各自的缓冲区，GUI 把编辑传给 proxy，保存由 proxy 执行。因此保存请求与内容版本必须一致。[Lapce 架构说明](https://docs.lapce.dev/development/architecture)

**事实。** `Buffer::save` 在写入前拒绝只读文件和不匹配的 revision，并解析符号链接。该实现对已有文件先复制 `.bak`，随后直接 truncate/write 原目标，成功后删除备份；它不是 RUPORA 当前使用的“写临时文件、同步、原子替换”算法，也不能据此宣称具备同等崩溃一致性。[Lapce 保存实现](https://github.com/lapce/lapce/blob/b604d57de4a820006d335a3be0d7583eb8fab558/lapce-proxy/src/buffer.rs#L65)

**适合复用。** 借鉴“版本不匹配必须拒绝”的边界，而不是引入 proxy 或复制备份写法。RUPORA 已有 document ID、snapshot token、旧结果拒绝和原子保存；下一步应验证这些契约在切换文档、保存为、更改路径、Undo 后内容重新相同等序列中仍成立。[RUPORA 文档版本接口](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/native/src/document.rs#L274)

**框架旁证及限制。** Floem 官方 editor 源码文档用 `owns_preedit` 表达组合归属，并在修改 preedit 前推进缓存版本；注释附有日文输入重现。其 MockWindow 明确不支持 IME 更新。这支持“组合归属/缓存顺序需要测试”和“无窗口测试不能证明系统输入法可用”两个设计判断。它是 2026-09-24 读取的 Floem 官方生成文档，未锁定到上述 Lapce 提交实际依赖的 Floem 版本，不能作为 Lapce 已覆盖该测试的证据。[Floem editor 源码文档](https://lapce.dev/floem/src/floem/views/editor/mod.rs.html)、[Floem MockWindow](https://lapce.dev/floem/src/floem/window/mock.rs.html)

## 5. RUPORA 已有能力与具体差距

下面“未看到”仅指列出的源码与工作流，不能排除维护者在仓库之外执行了人工验证。

| 领域 | 基线中已有的证据 | 可明确指出的差距 |
| --- | --- | --- |
| 编辑序列 | 产品测试对真实 App/egui 帧断言导航、输入、删除、IME、模式切换和历史；混乱输入回归使用 6 份初始文档、3 个固定种子、每次 64 步，并检查 Undo/Redo 往返。 | 此随机序列每帧投递一个事件，未生成格式快捷键或 IME 生命周期。其他批次回归覆盖了特定实例，但这不等于系统生成了事件分组、焦点和后台结果交错。 |
| 属性测试 | 任意 Unicode 格式往返、解析/块索引、表格和三方合并性质，每项配置 96 cases。 | `failure_persistence: None` 关闭样例自动持久化；已存在人工迁移的最小回归，但未看到统一的种子/最小输入收集并回放流程。 |
| fuzz | 三个 libFuzzer 目标；每周及手动 CI 对每个目标分配 30 秒，失败 artifacts 保留 14 天；已有崩溃回归进入普通测试。 | 不是持续长时间 fuzz。projection 目标一次只修改一处，不经过 App/egui、IME 或历史；其源码切分长度由单字节控制，实际 source 最长为 256 字节，256 KiB 总输入上限主要可用于 replacement。 |
| 视觉与可访问性 | 有真实帧 AccessKit 节点检查、shape/galley/坐标断言、窄窗/活动与静态块测试；PDF 有像素与结构回归。 | 导出 PDF 验证不是编辑器视觉验证。已检查的工作流未设置真实窗口的系统 IME 验收，也没有上述组合状态的固定图片基线。 |
| 数据安全 | 临时文件同目录写入并同步，再 persist/persist_noclobber；Unix 同步父目录；注入替换前失败和并发目标创建；恢复包含损坏/版本/预算/路径测试。 | 现有注入点很有价值，但不足以覆盖保存/恢复所有中断阶段、真实进程被终止、磁盘故障及平台差异。不能把普通进程退出测试描述成断电证明。 |
| 发布可信性 | 三平台 CI 运行 all-targets 测试、Clippy 和 release build；有 perf/依赖门禁。发布含 6 平台清单、SBOM、校验和、签名更新清单、来源证明、已发布版本不可覆盖和草稿完成后公开。 | 工作流允许开发者未签名安装包，OS 签名取决于配置。未看到安装后编辑/保存/恢复/升级的自动验收；校验和、签名、来源证明也不等于独立重建后的逐字节可重复性。 |

对应基线证据：[编辑序列](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/native/src/product_input_tests.rs#L534)、[App 批次回归](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/native/src/app_tests.rs)、[属性测试](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/tests/properties.rs#L9)、[projection fuzz](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/fuzz/fuzz_targets/wysiwyg_projection.rs)、[GUI 场景](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/tests/gui_scenarios.rs)、[段落布局测试](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/native/src/paragraph_layout_tests.rs)、[导出视觉测试](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/tests/export_visual.rs)、[原子保存](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/native/src/document.rs#L1235)、[恢复模块](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/native/src/recovery.rs)、[原生 CI](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/.github/workflows/native-ci.yml)、[发布流程](https://github.com/qinyin233/rupora/blob/214439180f70785711c10576afcf05114493f07c/.github/workflows/native-release.yml)。

## 6. 建议的实施顺序与验收条件

这些是针对 RUPORA 的工程建议，不声称各上游已经逐项实现，也不在本研究中把它们标记为完成。

### P0：扩展当前回放测试，而非创建第二个编辑器

1. **可保存的输入轨迹。** 在现有测试 harness 上记录初始源码、文档 ID、模式、双端光标、帧时间、RawInput 事件及分帧边界。失败输出种子、步骤和最小轨迹；已确认的最小失败样例进入普通 CI。
2. **有语义的序列生成。** 分别生成普通输入、Paste、导航/选区、多个格式快捷键、删除、历史、完整 IME 生命周期、焦点/文档切换；不要把任意无效 IME 事件当成真实操作系统协议。批次与分帧比较须保持事件顺序，并先说明哪些行为应等价。
3. **比较不只是正文。** 检查有方向的光标/选区、目标文档、隐藏标记完整性、当前投影、取消后状态和 Undo/Redo。对智能配对、链接粘贴、代码字面量给出固定预期，避免两条同样错误的路径互相通过。
4. **显式覆盖现有限制。** 包括多个命令同帧、额外 pass 不可用、弹窗/失焦/鼠标混合事件。内容不丢只是底线；格式命令必须在正确位置生效。IME 阶段与基于时间的历史合并不必在任意分帧方式下等价，应分别定义并检查它们的契约。

**验收：** 至少保留一个被新驱动发现的真实失败样例及修复前后证据；相同种子/轨迹在固定配置上可重放；驱动调用生产路径，测试自身不重写 TextEdit 行为。只增加随机步数不满足验收。

### P0：把文档身份与数据安全纳入序列

使用现有 snapshot token 和后台结果注入边界，枚举“发起任务 → 编辑/Undo/保存为/切换/关闭 → 结果返回”的顺序。验证过期结果被拒绝、结果不会写进另一个文档、用户草稿仍可取回。保存/恢复按写入、同步、替换、状态更新等边界注入可控失败，再单独增加子进程终止后的恢复用例。

**验收：** 各失败阶段有明确允许状态，例如目标保留完整旧内容或完整新内容，dirty/路径/历史与实际提交结果一致，未提交草稿仍可恢复。对子进程终止、文件系统报错、实际断电分别标记证据等级，不泛化结论。保持现有原子保存策略，不为照搬 Lapce 改为原地截断写入。

### P1：建立失败样例闭环，扩大 fuzz 的有效状态空间

- 为属性测试选择可复现的 seed/最小输入归档方式，或恢复合适的 failure persistence；正常 PR 不自动更新“正确答案”。
- projection fuzz 的 source/replacement 使用更合理的长度编码；新增多步编辑状态，明确资源上限。仅凭 `String` 的末尾必为字符边界不能证明编辑语义正确，应增加投影/光标/源码不变量。
- 将普通 CI 的短回归与独立、预算受控的较长 fuzz 活动分开；记录目标提交、工具链、实际时长、语料和崩溃样例。语料上传需限制体积，并避免使用真实私人文档。

**验收：** 人为注入已知错误后，活动能报错并产出可本地重放的最小样例；样例移入常规测试后阻止同类回归。运行时间或覆盖率是观测指标，不单独作为正确性证明。

### P1：原生 IME 与编辑器视觉的小规模验收矩阵

保留快速语义/几何测试；为组合文字范围、窄宽换行、活动/静态块一致性设置少量局部图片基线，固定字体、缩放、后端和主题。另制定实际原生窗口测试矩阵：中文预编辑/候选确认/取消、emoji 邻接、日文或韩文组合、切换模式/文档、失焦和撤销。优先覆盖项目正式支持且维护者能实际运行的 OS/输入法组合，记录版本与步骤；GUI 自动化无法稳定控制候选窗时保留明确的人工验收记录。

**验收：** 图片变化必须能定位到具体状态，调整阈值后验证仍能发现人为制造的错误。原生验收须从系统 IME 进入应用，不能把注入 `Event::Ime` 记作此项已完成。

### P1：以最终安装包执行用户路径

在发布前下载或取得待发布的实际产物，校验资产/更新清单，安装或解包后启动。执行“打开含中文和图片的 Markdown → WYSIWYG 修改 → 保存 → 重启核对 → 恢复未保存草稿”；有升级流程的平台另测保留用户数据。每个平台/架构的证据只覆盖实际运行的产物。不要把“在构建目录启动开发程序”算作安装包验收。

**验收：** 测试记录明确包含版本、资产 SHA-256、OS/架构和结果，发布依赖相应关卡。签名状态对用户可辨识；无证书的开发包不能标成 OS 可信签名包。若未来声称可重复构建，必须另行完成独立环境重建和二进制差异解释，现有 provenance 不替代该证据。

## 7. 暂不建议的移植与判断标准

- 不为可靠性目标重写 GUI 框架、照搬 Zed 整套调度器、引入 Lapce 远程 proxy；先在现有模块边界补可控输入与结果注入。
- 不通过隐藏 panic、丢弃尚未处理事件、放宽快照阈值、自动接受图片更新、跳过失败种子或缩小生成范围来获得绿灯。
- 不以新增测试数量、fuzz 构建成功、三平台编译成功或研究报告完成，宣布原生输入法与所有编辑序列已正确。

“可信可用”的可审查结果应是：已知失败有最小回归；输入、保存和恢复的契约清楚；失败可重放；关键平台与最终产物有实际验证；未验证边界被准确记录。这些实践能持续降低回归风险，不能证明软件不存在未知 bug。
