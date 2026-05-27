# 架构决策 0003：权限与分发边界

日期：2026-05-27

## 状态

已接受。

## 背景

DroidLoom 需要使用 Android 上的敏感能力：

- AccessibilityService：用于屏幕语义和 UI 动作。
- 可选 MediaProjection：用于像素级捕获。
- 可选 NotificationListenerService：用于通知读取和通知 action。
- 本地 trace：可能包含 App 名称、可见文本和动作历史。

Google Play 对 AccessibilityService 自动化有严格政策。一个通用的自主助手如果读取屏幕并执行动作，除非被限定为合规的无障碍工具或收窄为明确的确定性自动化，否则可能与 Play 政策冲突。

## 决策

DroidLoom 先作为 research/prototype/internal/sideload 项目启动。在完成产品范围和政策审查前，不宣称 Google Play 兼容。

运行时规则：

- 每个敏感能力都需要明确的用户 onboarding 和撤销路径。
- 活跃 Agent session 期间必须显示 foreground notification。
- 默认本地运行。
- 截图默认短生命周期，不持久化。
- 高风险动作需要用户确认。
- 工作流在执行前必须声明能力和副作用。

## 理由

- 避免围绕错误的政策假设构建产品。
- 让第一阶段工程目标聚焦在安全、可观察的运行时。
- 把合规要求放进架构，而不是实现后再补。

## 影响

正向影响：

- 降低误建静默自动化或过宽权限自动化的风险。
- 对贡献者而言边界更清晰。
- 更容易把政策约束加入编译器和运行时。

负向影响：

- 公开分发会推迟。
- 一些理想的自主用例必须保持 opt-in 或暂不支持。
- 用户测试需要先走侧载或内部测试分发路径。

## 后续事项

- 第一个 Android release 前增加权限 disclosure checklist。
- 在工作流 schema 中加入风险分类。
- 任何商店上架工作前都要重新评估 Play policy。
