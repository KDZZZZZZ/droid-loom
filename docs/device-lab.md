# 共享 Android 设备实验室与真机 CI

日期：2026-05-27

## 1. 结论

如果团队里有人没有 Android 手机，可以用一台专用手机搭建共享调试环境，同时让它承担部分 CI 测试。但它不能同时服务交互调试和 CI，必须用预约、锁和清理流程串行使用。

推荐方案：

- 日常开发：本地 JVM/unit tests、Android Emulator、GitHub-hosted runner。
- `dev` 分支：用 Android Emulator 调试和跑 CI，不占用共享真机。
- 无真机开发者：优先用 emulator；遇到无障碍、截图、通知和厂商 ROM 问题时，再使用 Android Device Streaming 或共享真机的远程镜像/ADB。
- DroidLoom 专项能力：用一台专用真机做 AccessibilityService、MediaProjection、通知、厂商 ROM 行为的 smoke test。
- CI：功能分支 PR 到 `dev` 跑模拟器；`dev` PR 到 `main` 由有权限的人审批后跑真机。
- 回归矩阵：等预算允许后接 Firebase Test Lab，而不是靠一台手机覆盖所有设备。

一句话：一台手机可以起步，但它是“设备实验室的最小形态”，不是完整测试矩阵。

## 2. 为什么需要真机

DroidLoom 与普通 Android App 不一样，它依赖系统级能力：

- AccessibilityService 读取节点树、执行 action、派发 gesture。
- MediaProjection 授权、截图、前台服务类型。
- NotificationListenerService 和通知 action。
- 前台 service、后台限制、厂商 ROM 权限管理。
- 本地 LLM 推理的真实 CPU/GPU/NPU、内存和发热行为。

这些能力在 emulator 上可以覆盖一部分，但无法替代真实设备。特别是厂商 ROM、权限弹窗、后台限制和真实触控/截图链路，必须有真机验证。

## 3. 团队无手机成员的开发路径

### 路径 A：Android Device Streaming

Android Device Streaming 由 Firebase 支持，可以在 Android Studio 中连接 Google 数据中心和 Android Partner Device Labs 的远程真机。官方说明中，它支持部署 App、查看显示、交互操作，并通过 ADB over SSL 接入。

适用：

- 没有本地 Android 手机的开发者。
- 需要快速看不同 Pixel、Samsung、OPPO、Xiaomi、vivo 等设备表现。
- 不想维护本地设备实验室。

限制：

- 需要 Android Studio 和 Firebase 项目权限。
- 会产生配额或计费问题。
- 对 DroidLoom 这类需要无障碍、屏幕捕获、通知权限的场景，仍要验证云端设备是否允许完整权限路径。
- 不适合作为所有 PR 的 required check。

### 路径 B：本地 emulator

适用：

- Compose UI。
- ViewModel、Repository、Room、DataStore。
- Workflow IR、compiler pass、policy guard。
- LlmEngine fake adapter。
- 一部分 UI Automator / instrumentation test。

限制：

- 无法代表厂商 ROM。
- 不适合验证真实端侧推理性能。
- 不适合作为 Accessibility/MediaProjection 可靠性的唯一依据。

### 路径 C：共享真机远程调试

搭一台“设备实验室主机”：

```text
开发者电脑
  -> VPN/SSH/Tailscale
  -> device-lab-host
      -> USB
      -> Android phone
```

主机安装：

- Android SDK platform-tools。
- JDK、Gradle、Android Studio 或命令行构建环境。
- `scrcpy` 或 Android Studio Device Mirroring。
- GitHub CLI、日志采集脚本。
- 可选 DeviceFarmer/STF，用浏览器管理多台设备；只有一台手机时 `scrcpy + SSH` 更简单。

远程访问方式：

- 交互看屏幕：`scrcpy`。
- IDE/命令行调试：通过 SSH tunnel 访问 ADB，或让开发者在主机上运行 Android Studio。
- 不要把 ADB 5037 或设备 5555 暴露到公网。

## 4. 一台手机同时做调试和 CI 的可行方案

可以，但必须串行。核心是“租约”：

```text
free -> reserved-by-human -> cleaning -> free
free -> reserved-by-ci -> cleaning -> free
```

建议约束：

- 工作时间优先人工调试。
- 夜间和午休跑 CI smoke。
- CI job 运行前检查设备空闲。
- CI job 运行中持有锁。
- job 结束后收集日志并清理设备。
- 连续失败自动下线设备，不继续污染后续测试。

需要两个锁：

- GitHub Actions `concurrency`：防止多个 workflow 同时抢设备。
- 主机本地锁：防止人工调试和本地脚本绕过 GitHub 并发控制。Linux 可用 `flock`，Windows 可用命名 mutex 或文件锁。

## 5. GitHub Actions 与自托管 runner 风险

GitHub 官方文档明确建议：self-hosted runner 只用于 private repository，因为 public repository 的 fork PR 可能通过 workflow 在自托管机器上执行危险代码。

DroidLoom 当前是 public repo，因此不建议把一台连着真机的自托管 runner 直接暴露给所有 PR。

采用 `dev -> main` 后，真机 runner 的触发条件应该更窄：

- 不跑 `feature/* -> dev`。
- 不跑 fork PR。
- 只跑同仓库 `dev` 到 `main` 的 PR。
- PR 必须经过有权限的人审批。
- workflow job 必须检查 `github.event.pull_request.head.repo.full_name == github.repository` 和 `github.event.pull_request.head.ref == 'dev'`。
- 最好叠加 protected environment required reviewers。

可选方案：

| 方案 | 用法 | 优点 | 风险 |
| --- | --- | --- | --- |
| Public repo 只跑 GitHub-hosted CI | PR required checks | 安全、简单 | 不能接本地真机 |
| Self-hosted runner 只跑 `dev -> main`、`workflow_dispatch` 和 trusted push | 手动/主线 smoke | 起步快 | 仍要保护 workflow、branch 和 environment |
| 私有 mirror repo 跑真机 CI | public repo 合并后同步到 private CI | 隔离 public PR 风险 | 多一个仓库和同步流程 |
| Firebase Test Lab | 云端真机/虚拟设备 | 设备矩阵、无需自建 | 成本、权限路径不一定完整 |

推荐当前阶段：

1. Public repo：`feature/* -> dev` 只设置 GitHub-hosted required checks 和 emulator smoke。
2. 共享真机：作为人工调试和 `dev -> main` smoke。
3. M2 后：新增受控 self-hosted runner，只接受同仓库 `dev -> main`。
4. Release candidate：Firebase Test Lab + 本地真机双跑。

## 6. 最小设备实验室配置

硬件：

- 一台 Android 11+ 手机，优先 Pixel 或接近 AOSP 的设备。
- 一台常开主机：Linux mini PC、NUC、Mac mini 或 Windows 主机。
- 稳定 USB 线和供电 USB hub。
- 有线网络优先。
- 手机散热和固定支架。

手机设置：

- 开发者选项。
- USB debugging。
- Stay awake while charging。
- 禁用锁屏或使用测试专用锁屏策略。
- 关闭自动系统更新。
- 使用测试 Google 账号，不登录个人账号。
- 禁止存放真实聊天、相册、联系人、短信。

Android 官方提供 `adb shell cmd testharness enable`，用于把测试设备恢复到更适合自动化测试的状态；它会禁用锁屏、关闭自动同步、关闭自动系统更新等。这个命令会重置设备，适合 nightly 或设备污染后使用，不适合每个 PR 都跑。

## 7. 真机 CI 流程

建议 smoke job：

```text
checkout
  -> build debug APK and androidTest APK
  -> acquire device lock
  -> adb devices / get-state
  -> collect device metadata
  -> install APK
  -> grant debug permissions / enable test hooks
  -> run connectedDebugAndroidTest
  -> collect logcat, screenshot, screenrecord, bugreport on failure
  -> uninstall / pm clear / reset settings
  -> release lock
```

示例 workflow 形态：

```yaml
name: Physical Device Smoke

on:
  pull_request:
    branches: [main]
  workflow_dispatch:
  schedule:
    - cron: "0 18 * * *"

permissions:
  contents: read

concurrency:
  group: physical-android-device
  cancel-in-progress: false

jobs:
  smoke:
    if: >
      github.event_name != 'pull_request' ||
      (github.event.pull_request.head.repo.full_name == github.repository &&
       github.event.pull_request.head.ref == 'dev')
    runs-on: [self-hosted, android-physical, trusted]
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@v6
      - uses: actions/setup-java@v4
        with:
          distribution: temurin
          java-version: "17"
      - uses: gradle/actions/setup-gradle@v6
      - name: Build
        run: ./gradlew :app:assembleDebug :app:assembleDebugAndroidTest
      - name: Run on physical device
        run: ./scripts/ci/run_physical_smoke.sh
```

这个 workflow 不应作为 public PR 的默认 required check。它适合 `dev -> main`、手动、夜间或 trusted branch。

## 8. Accessibility/MediaProjection 测试策略

不要把所有系统权限路径都放进真机 CI 的 required gate。

分层：

| 能力 | PR 必跑 | 真机 smoke | 手工/RC |
| --- | --- | --- | --- |
| Workflow IR / compiler | 是 | 否 | 是 |
| LlmEngine fake adapter | 是 | 否 | 是 |
| Accessibility node parsing | 是，可用 fixture | 是 | 是 |
| Accessibility action executor | 部分 mock | 是 | 是 |
| Gesture fallback | 否 | 是 | 是 |
| MediaProjection prompt | 否 | 可选 | 是 |
| NotificationListener | 否 | 可选 | 是 |
| 端侧 LLM 性能 | 否 | nightly | 是 |
| 厂商 ROM 行为 | 否 | 多设备/手工 | 是 |

对无障碍服务启用、MediaProjection 授权、通知访问这类系统页面，不要完全依赖自动点击。建议：

- debug build 中提供测试入口和诊断页面。
- 对纯逻辑使用 fake service 和 fixture。
- 真机 smoke 只跑最小闭环。
- release candidate 做人工确认清单。

## 9. 远程调试使用规范

共享真机必须有使用纪律：

- 使用前预约。
- 使用后执行清理脚本。
- 不登录个人账号。
- 不测试真实支付、真实聊天、真实联系人。
- 调试时避免采集真实屏幕内容到仓库 artifact。
- adb、scrcpy、STF 只在 VPN 或 SSH tunnel 内访问。
- 每次 CI 失败保留最小必要日志，截图默认脱敏或不上传。

推荐保留 artifacts：

- `adb logcat`，按 package 过滤。
- `adb shell dumpsys window`。
- `adb shell dumpsys accessibility`。
- 测试报告 XML/HTML。
- 失败时短 screenrecord，仅限测试 App 和系统设置页面。

不保留：

- 真实用户 App 截图。
- 通知正文。
- 联系人、短信、相册、聊天记录。
- 模型 API key 或 signing key。

## 10. 当前路线图建议

M1：

- 先不接真机 CI。
- 文档规定设备实验室要求。
- Android 工程支持 emulator/unit tests。
- `dev` 作为模拟器调试和普通 CI 分支。

M2：

- 搭一台共享真机。
- 建立手动 smoke 脚本。
- 让无手机开发者通过 scrcpy/SSH 调试。
- `dev -> main` 开始接入受控真机 smoke。

M3：

- 加入 nightly 真机 smoke。
- LlmEngine 使用 fake/small model。
- 收集冷启动、基础内存和 action 成功率。

M4+：

- 私有 mirror 或受控 self-hosted runner。
- Firebase Test Lab 做 release candidate 设备矩阵。
- 多设备后再引入 DeviceFarmer/STF。

## 11. 参考资料

- Android Device Streaming：https://developer.android.com/studio/run/android-device-streaming
- Android Debug Bridge：https://developer.android.com/tools/adb
- Android continuous integration：https://developer.android.com/studio/projects/continuous-integration
- Firebase Test Lab CI：https://firebase.google.com/docs/test-lab/android/continuous
- GitHub-hosted runner Android hardware acceleration：https://docs.github.com/en/actions/reference/runners/github-hosted-runners
- GitHub self-hosted runner：https://docs.github.com/en/actions/concepts/runners/self-hosted-runners
- GitHub self-hosted runner security warning：https://docs.github.com/en/actions/how-tos/manage-runners/self-hosted-runners/add-runners
- scrcpy：https://github.com/Genymobile/scrcpy
- DeviceFarmer/STF：https://github.com/DeviceFarmer/stf
