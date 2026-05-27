# 产品能力边界

日期：2026-05-27

## 1. 参考对象：AutoGLM 的边界启发

AutoGLM-Phone 公开资料给 DroidLoom 的主要启发不是“让模型完全接管手机”，而是把 Phone Use 产品能力拆成几个明确边界：

- 有限动作空间：`Launch`、`Tap`、`Type`、`Swipe`、`Back`、`Home`、`Long Press`、`Double Tap`、`Wait`、`Take_over`。
- 任务入口是自然语言，但执行过程被约束为可枚举动作。
- 登录、验证码等场景触发人工接管。
- 敏感操作通过 callback 或确认机制暂停执行。
- 支持 50+ 高频中文 App，但不是宣称对所有 App、所有页面都可靠。
- Open-AutoGLM 的工程路径依赖 ADB/HDC 和外部模型服务，适合研究、开发和远程调试。
- AutoGLM 2.0 方向强调云手机沙箱、动作可回放、可审计、可干预，并把真实敏感数据隔离在用户物理环境之外。

DroidLoom 的差异：

- DroidLoom 的目标是 Android 原生 App，不把 ADB、root、shell 注入作为普通用户运行时前提。
- DroidLoom 默认本地推理、本地 trace、本地权限控制；远端模型必须显式开启。
- DroidLoom 不是只做屏幕像素控制，还要把 Android API、Intent、Shortcut、通知 action、无障碍节点动作和手势兜底统一编排。
- DroidLoom 的核心研究点是工作流 IR 与编译优化，而不是仅输出下一步坐标动作。

## 2. 能力等级

### L0：只读观察

允许：

- 读取当前屏幕 accessibility tree。
- 在用户授权后截取当前屏幕或局部区域。
- 对截图做本地 OCR。
- 展示当前页面摘要、可操作控件和风险提示。

不允许：

- 自动点击、输入、发送、购买、删除或授权。
- 静默保存截图。
- 静默上传屏幕内容。

### L1：建议模式

允许：

- 根据目标和屏幕状态给出下一步建议。
- 高亮目标控件或生成操作说明。
- 解释当前页面中哪些控件可用、哪些动作有风险。

不允许：

- 在没有用户明确确认时执行动作。
- 绕过系统权限弹窗或应用登录/验证流程。

### L2：确定性工作流

允许：

- 执行用户预先安装或临时确认的工作流。
- 工作流必须有能力声明、动作摘要和风险等级。
- 动作后必须观察新状态并运行 verifier。

典型场景：

- 打开某个 App 并进入指定页面。
- 批量整理无风险信息。
- 搜索、筛选、收藏、播放、导航查询等低风险任务。

限制：

- 工作流不能隐藏副作用。
- 工作流不能动态扩大权限。
- 失败时进入 bounded retry，超过阈值后暂停。

### L3：受限 Agent 执行

允许：

- 在支持的 App 和支持的动作集合内，由 Agent 规划下一步。
- Agent 只能调用工具注册表中声明的 action。
- Planner 输出必须经过 schema parse、policy guard 和 verifier。

限制：

- 必须有 session foreground notification。
- 必须有实时暂停、接管和停止入口。
- 必须记录可审计 trace。
- 只对明确支持的 App/任务类型承诺可靠性。

### L4：高风险动作确认

允许：

- Agent 可以准备高风险动作，但不能直接提交。
- 用户确认后才可以继续。

高风险动作包括：

- 支付、转账、红包、金融交易。
- 购买、下单、订票、订酒店。
- 发送公开内容或私聊消息。
- 删除、清空、撤回、取消订单。
- 授权、登录、绑定账号、修改隐私设置。
- 访问联系人、相册、短信、文件、企业内部系统。

必须要求：

- 操作前展示动作摘要、目标 App、目标对象、金额/内容/接收人等关键参数。
- 用户使用系统级确认或 App 内显式确认。
- 验证执行结果并写入脱敏 trace。

### L5：人工接管

触发条件：

- 登录、验证码、人脸识别、指纹、二次验证。
- 支付密码、安全键盘、银行或政务 App。
- 权限弹窗含义不明确。
- 模型置信度不足或 verifier 连续失败。
- 页面包含高度隐私内容，且工作流没有明确授权。

行为：

- DroidLoom 暂停 Agent。
- 用户手动完成当前步骤。
- 用户确认后，Agent 重新观察状态并继续或结束。

## 3. 明确不做

近期不做：

- 静默后台常驻操作手机。
- 绕过验证码、登录、安全键盘、支付确认或系统权限弹窗。
- 自动读取并上传个人聊天、相册、短信、通讯录、企业数据。
- 在用户真实账号中做不可撤销操作且无需确认。
- 对所有 App 宣称通用可靠。
- 以 ADB、root、shell 权限作为普通用户产品依赖。
- 通过坐标点击执行高风险动作。

研究环境中可做但不进入用户产品默认路径：

- ADB/HDC 控制。
- 模拟器或云手机批量任务。
- 远端 VLM/LLM 服务。
- 大规模行为 trace 采集。

## 4. 产品承诺方式

DroidLoom 应该按“支持矩阵”承诺能力，而不是泛化承诺：

| 维度 | 示例 |
| --- | --- |
| 设备 | Android 版本、厂商 ROM、是否支持无障碍截图 |
| App | 包名、版本范围、页面范围 |
| 任务 | 搜索、播放、收藏、查询、填表、下单前确认 |
| 动作 | Intent、Accessibility action、Gesture fallback |
| 风险等级 | L0-L5 |
| 验证方式 | 文本匹配、节点状态、页面 URL/deep link、截图 OCR |

每个工作流发布前必须生成：

- 需要的权限；
- 会读取的数据；
- 会执行的动作；
- 高风险确认点；
- 支持的 App 和版本；
- 失败恢复方式；
- trace 保存策略。

## 5. 对架构的要求

为了落实上述边界，运行时必须提供：

- `TakeOver` action：任何模块都可以请求人工接管。
- `Confirm` action：高风险动作必须经过用户确认。
- `StopSession` action：用户随时停止。
- `TraceReplay`：动作链可回放和审计。
- `CapabilityManifest`：工作流能力声明。
- `RiskPolicy`：静态和运行时风险判定。
- `SupportedAppMatrix`：支持 App、页面和任务矩阵。
- `SandboxProfile`：研究模式、云手机模式、本机模式的能力差异。

## 6. 参考资料

- AutoGLM-Phone 官方文档：https://docs.bigmodel.cn/cn/guide/models/vlm/autoglm-phone
- Open-AutoGLM README：https://github.com/zai-org/Open-AutoGLM/blob/main/README_en.md
- AutoGLM 开源说明：https://autoglm.z.ai/blog/
