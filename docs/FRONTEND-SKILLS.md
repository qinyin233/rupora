# 前端 skill 评估与选型

评估日期：2026-09-12。目标是 RUPORA 原生 Rust + egui 编辑器的界面改进，延续用户要求的 Apple 风格。判断来自上游 SKILL.md、配套规则和实际检索结果，不以 star 数作为质量结论。

| 候选 | 优点 | 当前项目的限制 | 结论 |
| --- | --- | --- | --- |
| [Anthropic frontend-design](https://github.com/anthropics/skills/tree/main/skills/frontend-design) | 简洁、技术栈约束少；要求先确定色彩、字体、布局，再复核和实现；尊重既有审美要求 | 具体交互和无障碍规则较少 | **视觉规划首选，已安装** |
| [UI/UX Pro Max](https://github.com/nextlevelbuilder/ui-ux-pro-max-skill) | 可搜索的交互规则；明确关注焦点、状态、文本对比度、稳定布局；含桌面技术栈数据 | 没有 egui 专项；设计系统生成器两次给出营销页结构，不能直接当作编辑器方案 | **交互检查搭配，已安装** |
| [Impeccable](https://github.com/pbakaus/impeccable) | 设计、审查、打磨流程完整；Operate 模式重视任务效率；支持原生审查 | 当前检测器主要依赖 Web；原生专项偏 iOS/Android；引入额外运行器对本项目收益有限 | Web 项目完整工作流的强候选，本次未安装 |

## 固定来源

- frontend-design：`anthropics/skills@34040c9c568585f6929bedeaad110ad08f079624`，`skills/frontend-design`。
- ui-ux-pro-max：`nextlevelbuilder/ui-ux-pro-max-skill@7f69fed6a2717900085f1bc3b263721f8ba025e2`，`.claude/skills/ui-ux-pro-max`。
- Impeccable 评估快照：`cb56ed6c19a07329a9fa0cd4e657bee040156593`，`.agents/skills/impeccable`。

通过 Codex 自带 skill-installer 安装前两项到用户 skills 目录。本仓库不依赖 skill 的运行时，不加入远程字体、Web 框架或浏览器扩展。

## 实际试用与取舍

Pro Max 的 `desktop writing editor minimal` 和 `note taking productivity app` 两次 design-system 检索分别返回单列营销页、产品演示页，配色也偏离现有产品，因此没有将它们持久化为项目设计系统。最终设计使用本项目的界面事实与用户偏好。

`keyboard focus visible --domain ux` 返回适用于所有平台的 Focus States；`stable layout overflow --domain ux` 返回溢出、长文本及容器适配规则。应用其可验证的行为要求，在 egui 中独立实现。移动端触控尺寸、CSS、GSAP、浏览器专属规则不机械套用到桌面程序。

具体视觉约定见根目录 [DESIGN.md](../DESIGN.md)。验证包括桌面明暗主题、窄分栏、长文件名、键盘焦点和既有输入回归。
