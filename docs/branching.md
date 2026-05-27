# 分支管理规范

日期：2026-05-27

## 1. 总原则

DroidLoom 采用 `dev -> main` 两级门禁：

```text
feature/*, fix/*, docs/*, research/*
  -> PR 到 dev
  -> GitHub-hosted CI + Android Emulator + 单元测试

dev
  -> PR 到 main
  -> 有权限的人审批
  -> 共享真机 / self-hosted runner smoke
  -> main
```

这个模型的目的：

- `dev` 承担日常集成和模拟器调试，不占用真机。
- 没有 Android 手机的开发者也能通过 emulator、fixture、fake `LlmEngine` 完成大部分开发。
- `main` 只接收从受保护 `dev` 发起的 PR，进入主线前必须跑真机 smoke。
- 真机 runner 不暴露给任意 public fork PR。

## 2. 分支类型

| 分支 | 用途 | 生命周期 | 示例 |
| --- | --- | --- | --- |
| `main` | 稳定主线和发布入口 | 长期 | `main` |
| `dev` | 日常集成、模拟器调试、进入 main 前的候选分支 | 长期 | `dev` |
| `feature/*` | 新功能 | 短期，合入 `dev` | `feature/workflow-ir-parser` |
| `fix/*` | 缺陷修复 | 短期，合入 `dev` | `fix/a11y-node-bounds` |
| `docs/*` | 文档改动 | 短期，合入 `dev` 或必要时直接 PR 到 `main` | `docs/ci-plan` |
| `research/*` | 调研资料、ADR、实验记录 | 短期到中期，合入 `dev` | `research/mlc-android-profile` |
| `spike/*` | 可丢弃技术验证 | 严格限时，不直接合入 | `spike/llama-gguf-android` |
| `release/*` | 发布准备 | 有 Android artifact 后启用 | `release/0.1.0` |
| `hotfix/*` | 已发布版本紧急修复 | 有 release 后启用 | `hotfix/0.1.1-a11y-crash` |

分支命名要求：

- 使用小写字母、数字和连字符。
- 前缀必须来自上表。
- 名称表达目标，不写人名或模糊词。
- 一个分支只解决一个主题。

## 3. `dev` 保护策略

`dev` 是模拟器调试和日常集成分支，应保护但保持迭代速度：

- Require a pull request before merging。
- Require status checks before merging。
- Require conversation resolution before merging。
- Require linear history。
- Do not allow force pushes。
- Do not allow deletions。
- 至少 1 名 reviewer。
- 允许 squash merge。

`dev` required checks：

- `docs / markdown-hygiene`
- `android / lint-test-build`
- `android / emulator-smoke`
- `workflow / unit-property-tests`，进入 M4 后启用

`dev` 不跑真机 runner。真机是稀缺资源，只用于 `dev -> main`。

## 4. `main` 保护策略

`main` 是稳定主线和发布入口，比 `dev` 更严格：

- 只接受从同仓库 `dev` 发起的 PR，release/hotfix 流程启用后再允许例外。
- Require a pull request before merging。
- Require status checks before merging。
- Require approvals from CODEOWNERS 或指定维护者。
- Require conversation resolution before merging。
- Require linear history。
- Do not allow force pushes。
- Do not allow deletions。
- 不允许普通贡献者直接 push。

`main` required checks：

- `docs / markdown-hygiene`
- `android / lint-test-build`
- `android / emulator-smoke`
- `device / physical-smoke`
- `security / dependency-review`，Android 工程落地后启用

真机 smoke 只在以下条件下运行：

- PR base 是 `main`。
- PR head 是同仓库的 `dev`。
- PR 已由有权限的人发起或审批。
- workflow 使用 protected environment 或 trusted runner label。

## 5. PR 规则

每个 PR 必须说明：

- 改了什么。
- 为什么改。
- 如何验证。
- 是否影响权限、隐私、Agent 动作或工作流行为。

从功能分支到 `dev`：

- 目标是快速集成。
- 主要依赖 GitHub-hosted CI、emulator、unit tests。
- 不要求真机。

从 `dev` 到 `main`：

- 目标是稳定主线。
- 必须由有权限的人审批。
- 必须跑共享真机 smoke。
- 如果变更涉及权限、真机行为、端侧推理或安全边界，PR 描述必须列出风险和回滚方式。

涉及以下内容必须补文档或 ADR：

- Android 敏感权限。
- Agent 能力边界。
- LLM 后端接口。
- 工作流 IR 或 compiler pass。
- Trace、日志、截图、隐私数据保存策略。
- CI、发布、签名、依赖安全策略。

## 6. 合并策略

默认使用 squash merge：

- PR 标题作为最终提交标题。
- PR 描述保留关键背景、验证方式和风险。
- 合并后删除源分支。

例外：

- `dev -> main` 可以保留 merge commit，方便看每次进入主线的集成点。
- release 分支合并可以保留 merge commit。
- 需要保留详细 bisect 历史的低层 runtime 改动，可以使用 rebase merge。

禁止：

- 直接 push 到 `main`。
- 对 `dev` 或 `main` 使用 force push。
- 把模型文件、APK/AAB、私钥、token、截图数据直接提交到仓库。

## 7. Commit 信息

推荐使用 Conventional Commits 风格：

```text
docs: add CI plan
feat: add workflow IR parser
fix: handle missing accessibility root
test: add prompt segment liveness tests
ci: add Android lint workflow
chore: update Gradle wrapper
```

提交粒度：

- 一个提交表达一个完整意图。
- 格式化和行为改动分开。
- 大规模重命名和逻辑改动分开。

## 8. Release 与 Tag

在 Android App 可运行前，不启用正式 release 流程，只使用 `dev -> main`。

进入可安装阶段后：

- 版本号使用 SemVer：`v0.1.0`、`v0.1.1`。
- 每个 release tag 对应一个可追溯构建。
- debug/internal artifact 可通过 GitHub Actions artifact 保存。
- 公开 release 必须包含 changelog、权限变化说明和已知风险。

## 9. 冲突处理

功能分支同步 `dev`：

```bash
git fetch origin
git switch feature/my-work
git rebase origin/dev
```

冲突解决后：

```bash
git add <resolved-files>
git rebase --continue
git push --force-with-lease
```

只允许对自己的短生命周期分支使用 `--force-with-lease`。不要 force push `dev`、`main` 或多人共享分支。

## 10. 参考资料

- GitHub protected branches: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches
- GitHub rulesets: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/about-rulesets
- Pro Git book: https://git-scm.com/book
