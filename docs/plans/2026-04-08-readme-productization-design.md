# README 多语言产品化重构 Design

**日期：** 2026-04-08
**范围：** `README.md`、`README_EN.md`、`README_JA.md`、`README_KO.md`、`README_RU.md`

## 目标

将 README 从“功能堆叠型说明文档”升级为“现代产品化入口页”，同时保留开发者可信度与完整信息密度。

核心成功标准：
1. 第一屏观感更高级，提升品牌感与传播性（C）
2. 降低阅读门槛，提升 Star 转化（A）
3. 多语言 README 全部保持完整版，不做精简版分层

## 受众策略

- **主受众：开发者（B）**
- 同时覆盖：普通用户（快速下载体验）与合作/传播受众（项目级别感）

## 风格方向

- 现代、产品化、开发者体验强（Vercel 风格）
- 信息架构“前轻后重”：首屏讲价值，后文讲技术与扩展
- 语义等价翻译，不做逐句机械直译

## 信息架构（统一模板）

1. Hero（标题 + 副标题 + CTA）
2. Why Kokoro Engine（3 条差异化价值）
3. Screenshots（主图 + 次图）
4. Quick Start（下载优先，源码次之）
5. Core Capabilities（6 类能力卡片化）
6. Architecture for Developers（简图 + 三层说明 + docs 链接）
7. Extensibility（MOD / MCP / Telegram）
8. Roadmap（Now / Next）
9. Contributing
10. Community
11. Sponsor
12. License（MIT + Live2D 声明）

## 内容原则

- 首屏避免大段文字，优先价值主张与导航入口
- 长功能列表改为能力分组，降低认知负担
- 增加信任信号：技术栈、架构入口、发布下载、许可证边界
- 各语种完整保留同构章节，术语统一（MOD、MCP、Offline-first）

## 非目标

- 本轮不改代码、不改截图资源、不改 docs 架构文档正文
- 本轮不引入自动化翻译流水线脚本（仅先完成内容重构）

## 验收标准

- 5 份 README 全部采用统一新结构
- 5 份 README 均含完整章节（非简版）
- 跨语言互链、Release 链接、Sponsor 链接、License 链接可用
- 每份 README 均保留开发者入口（Architecture、Quick Start、Contributing）
