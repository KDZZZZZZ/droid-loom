# CI 设计与调研

日期：2026-05-27

## 1. 结论

DroidLoom 的 CI 按 `dev -> main` 两级门禁设计：

- 功能分支 PR 到 `dev`：跑 GitHub-hosted CI、Android Emulator、unit tests、lint、debug build。`dev` 就是模拟器调试和日常集成分支。
- `dev` PR 到 `main`：由有权限的人审批后，跑共享真机 smoke。真机 runner 只接受同仓库 `dev` 到 `main` 的受控 PR。
- `main`：保持稳定，后续承担 release candidate、tag 和 artifact 产出。

真机测试单独按设备实验室管理，见 [共享 Android 设备实验室与真机 CI](./device-lab.md)。一台手机可以起步，但只适合 `dev -> main`、manual、nightly 或 trusted branch smoke，不适合作为 public fork PR 的默认 required check。

推荐阶段：

1. M0 当前阶段：Markdown hygiene、内部链接检查。
2. M1 Android 外壳：`feature/* -> dev` 跑 Gradle wrapper validation、lint、unit test、debug build。
3. M2 屏幕观察与动作执行：`feature/* -> dev` 跑 emulator smoke；`dev -> main` 跑真机 smoke。
4. M3 LLM 后端：`dev` 跑 fake/small adapter tests；`main` gate 跑真机 profile smoke，真实大模型文件不进仓库。
5. M4+ 工作流编译器：IR parser、compiler pass、liveness/property tests 作为 `dev` 和 `main` required checks。

## 2. 官方资料要点

GitHub Actions：

- GitHub Actions 可以直接在仓库里定义 CI workflow。
- 对 Java/Gradle 项目，官方模板会 checkout、设置 JDK、设置 Gradle 环境，并通过 Gradle Wrapper 执行 build。
- `gradle/actions/setup-gradle` 会处理 Gradle User Home 缓存和执行摘要。
- GitHub-hosted runner 是干净环境，依赖缓存能减少下载和构建时间。
- cache 不应保存 token、登录凭证等敏感信息。
- `GITHUB_TOKEN` 应按最小权限配置，常规 CI 默认 `contents: read`。

Android：

- Android 官方 CI 文档建议每次提交后自动 build/test。
- Android 测试可以在 CI 中使用 Android Emulator 或 Firebase Test Lab。
- 如果 CI 机器没有安装 Android Studio，需要用 `sdkmanager` 接受所需 SDK license。
- Firebase Test Lab 可用于真实设备或云端设备矩阵，更适合 nightly/regression，不建议所有 PR 默认跑大矩阵。

## 3. M0 Docs CI 建议

当前仓库仍是 docs-first，建议优先新增 `.github/workflows/docs.yml`：

- 触发：push 到 `dev`/`main`、PR 到 `dev`/`main`、手动运行。
- 权限：`contents: read`。
- 检查：
  - Markdown 文件不能有 CRLF。
  - Markdown 文件不能有 trailing whitespace。
  - 相对链接必须指向存在的本地文件。

这是当前唯一建议设置为 required check 的 job：

```text
docs / markdown-hygiene
```

建议配置：

```yaml
name: Docs

on:
  pull_request:
    branches: [dev, main]
  push:
    branches: [dev, main]
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: docs-${{ github.ref }}
  cancel-in-progress: true

jobs:
  markdown-hygiene:
    name: markdown-hygiene
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - name: Check Markdown hygiene
        shell: bash
        run: |
          set -euo pipefail
          python - <<'PY'
          from pathlib import Path
          import re
          import sys
          from urllib.parse import unquote

          root = Path(".").resolve()
          files = sorted(p for p in root.rglob("*.md") if ".git" not in p.parts)
          failures = []
          link_re = re.compile(r"!?\\[[^\\]]*\\]\\(([^)]+)\\)")

          for path in files:
              raw = path.read_bytes()
              rel = path.relative_to(root).as_posix()

              if b"\\r\\n" in raw:
                  failures.append(f"{rel}: contains CRLF line endings")

              text = raw.decode("utf-8")
              for idx, line in enumerate(text.splitlines(), start=1):
                  if line.rstrip(" \\t") != line:
                      failures.append(f"{rel}:{idx}: trailing whitespace")

              for match in link_re.finditer(text):
                  target = match.group(1).strip()
                  if not target or target.startswith("#"):
                      continue
                  if re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", target):
                      continue
                  if target.startswith("//"):
                      continue

                  target = unquote(target.split("#", 1)[0])
                  candidate = (path.parent / target).resolve()

                  try:
                      candidate.relative_to(root)
                  except ValueError:
                      failures.append(f"{rel}: link escapes repository: {target}")
                      continue

                  if not candidate.exists():
                      failures.append(f"{rel}: missing linked file: {target}")

          if failures:
              print("\\n".join(failures))
              sys.exit(1)

          print(f"Checked {len(files)} Markdown files.")
          PY
```

## 4. M1 Android CI 设计

Android 工程生成后新增 `.github/workflows/android.yml`。

建议 workflow 负责 `dev` 的模拟器调试和普通 PR gate：

```yaml
name: Android

on:
  pull_request:
    branches: [dev]
  push:
    branches: [dev]
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: android-${{ github.ref }}
  cancel-in-progress: true

jobs:
  android:
    name: lint-test-build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6
      - uses: actions/setup-java@v4
        with:
          distribution: temurin
          java-version: '17'
      - uses: gradle/actions/setup-gradle@v6
      - name: Run lint and unit tests
        run: ./gradlew lintDebug testDebugUnitTest
      - name: Build debug APK
        run: ./gradlew assembleDebug
      - name: Upload debug APK
        if: github.event_name == 'push' && github.ref == 'refs/heads/dev'
        uses: actions/upload-artifact@v4
        with:
          name: debug-apk
          path: app/build/outputs/apk/debug/*.apk
```

说明：

- JDK 版本先用 Android Gradle Plugin 广泛支持的 17，等工程确定 AGP/Kotlin 版本后再调整。
- `setup-gradle` 当前主线已到 v6；Android 工程落地时可以 pin 到具体版本或 commit SHA。
- APK artifact 只在 `dev` push 后上传，PR 不默认产出可分发包。
- 不要在 PR workflow 中使用签名私钥或发布 token。

M2 后为 `dev` 增加 emulator smoke。可以使用 Gradle Managed Devices，或在 GitHub-hosted runner 上创建 AVD。示例任务名应由 Android 工程实际配置决定：

```bash
./gradlew pixel2Api35DebugAndroidTest
```

## 5. Instrumented Test 策略

Android instrumentation test 成本高、耗时长、容易受 emulator 环境影响。建议分层：

| 层级 | 触发 | 内容 | 是否 required |
| --- | --- | --- | --- |
| Unit/JVM | PR 到 `dev`、`dev` push、PR 到 `main` | IR、compiler pass、policy、prompt segment | 是 |
| Robolectric 或无设备测试 | PR 到 `dev`、`dev` push、PR 到 `main` | ViewModel、repository、纯 Android 逻辑 | 是 |
| Emulator smoke | PR 到 `dev`、`dev` push | Accessibility service 基础能力、简单 UI Automator | M2 后逐步 required |
| 真机 smoke | `dev` PR 到 `main` | Accessibility、Gesture、MediaProjection 最小闭环 | 是 |
| Firebase Test Lab | nightly、release candidate | 多设备、多 API、长链路测试 | release 前 required |
| 手工真机 | release candidate | 权限、厂商 ROM、系统设置、后台限制 | release 前 required |

早期不要把 emulator smoke 设成 `dev` 的 required check。等测试稳定、耗时可控后，再挑极少数 smoke case 加入 `dev` PR gate。

真机 smoke 放在 `dev -> main`，不接受来自 fork PR 的代码。由于本仓库是 public repo，自托管 runner 必须只对同仓库 `dev` 分支的受控 PR 开放。

## 6. LLM 与原生构建 CI

MLC LLM、llama.cpp、NDK、模型文件会显著拉长 CI。建议：

- Android App 的普通 PR 不下载大模型。
- 使用 tiny/mock model 或 fake `LlmEngine` 做单元测试。
- llama.cpp/MLC adapter 先做接口编译和 JNI smoke，不跑真实大模型。
- 真实模型 profile 放到 nightly/manual workflow。
- 不把 `.gguf`、`.safetensors`、APK、AAB、native `.so` 大产物提交到 git。
- 需要 native cache 时单独设计，避免污染普通 Gradle cache。

## 7. 安全策略

CI 安全规则：

- 默认 `permissions: contents: read`。
- 不使用 `pull_request_target` 执行不可信 PR 代码。
- 不在 PR workflow 暴露 signing key、cloud credential、model API key。
- secrets 只用于 protected environment 或手动 release job。
- release job 需要环境 required reviewers。
- 外部 GitHub Action 优先使用官方或可信维护者；关键发布链路 pin 到版本或 commit SHA。
- cache 不保存凭证、模型 license 文件、私钥或本地用户数据。

建议后续安全 workflow：

- `github/codeql-action`：Java/Kotlin 代码安全扫描。
- `actions/dependency-review-action`：PR 依赖变化审查。
- `gradle/actions/dependency-submission`：把 Gradle 依赖提交到 GitHub Dependency Graph。

## 8. Required Check 规划

当前：

- `docs / markdown-hygiene`

M1：

- `docs / markdown-hygiene`
- `android / lint-test-build`

M2-M4，`feature/* -> dev`：

- `workflow / unit-property-tests`
- `android / lint-test-build`
- `android / emulator-smoke`
- `security / dependency-review`

M2-M4，`dev -> main`：

- `workflow / unit-property-tests`
- `android / lint-test-build`
- `android / emulator-smoke`
- `device / physical-smoke`
- `security / dependency-review`

Release candidate：

- `instrumented / firebase-test-lab`
- `release / signed-artifact-dry-run`
- `security / codeql`

## 9. 参考资料

- GitHub Actions 文档：https://docs.github.com/en/actions
- GitHub Java with Gradle workflow：https://docs.github.com/en/actions/tutorials/build-and-test-code/java-with-gradle
- GitHub dependency caching：https://docs.github.com/en/actions/concepts/workflows-and-actions/dependency-caching
- GitHub cache reference：https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching
- GitHub protected branches：https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches
- GitHub `GITHUB_TOKEN` permissions：https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/controlling-permissions-for-github_token
- Android continuous integration：https://developer.android.com/studio/projects/continuous-integration
- Firebase Test Lab CI：https://firebase.google.com/docs/test-lab/android/continuous
- Gradle GitHub Actions：https://github.com/gradle/actions
