# 分支管理规范

日期：2026-05-27

## 1. 总原则

DroidLoom 采用轻量 trunk-based workflow：

- `main` 永远保持可读、可构建、可发布文档。
- 所有改动通过短生命周期分支进入 `main`。
- 通过 PR 做代码审查、CI 校验和设计讨论。
- 不使用长期 `develop` 分支，避免项目早期出现双主干。
- 大型探索用 `spike/*`，合并前必须整理成小 PR。

## 2. 分支类型

| 分支 | 用途 | 生命周期 | 示例 |
| --- | --- | --- | --- |
| `main` | 默认分支，稳定主线 | 长期 | `main` |
| `feature/*` | 新功能 | 短期，建议 1-5 天 | `feature/workflow-ir-parser` |
| `fix/*` | 缺陷修复 | 短期 | `fix/a11y-node-bounds` |
| `docs/*` | 文档改动 | 短期 | `docs/ci-plan` |
| `research/*` | 调研资料、ADR、实验记录 | 短期到中期 | `research/mlc-android-profile` |
| `spike/*` | 可丢弃技术验证 | 严格限时 | `spike/llama-gguf-android` |
| `release/*` | 发布准备 | 有 Android artifact 后启用 | `release/0.1.0` |
| `hotfix/*` | 已发布版本紧急修复 | 有 release 后启用 | `hotfix/0.1.1-a11y-crash` |

分支命名要求：

- 使用小写字母、数字和连字符。
- 前缀必须来自上表。
- 名称要表达目标，不写人名或模糊词。
- 一个分支只解决一个主题。

## 3. `main` 保护策略

建议在 GitHub ruleset 或 branch protection 中保护 `main`：

- Require a pull request before merging。
- Require status checks before merging。
- Require conversation resolution before merging。
- Require linear history。
- Do not allow force pushes。
- Do not allow deletions。
- 建议启用 squash merge，保持主线历史简洁。

初期必需检查：

- `docs / markdown-hygiene`

Android 项目生成后追加：

- `android / lint`
- `android / unit-test`
- `android / assemble-debug`
- `security / dependency-review`

注意：如果某个 required check 使用 `paths` 过滤，PR 未触发该 workflow 时可能导致 required check 一直缺失。更稳的做法是保留一个总是运行的轻量 gate job，再在 job 内判断是否需要执行重任务。

## 4. PR 规则

每个 PR 必须说明：

- 改了什么。
- 为什么改。
- 如何验证。
- 是否影响权限、隐私、Agent 动作或工作流行为。

涉及以下内容必须补文档或 ADR：

- Android 敏感权限。
- Agent 能力边界。
- LLM 后端接口。
- 工作流 IR 或 compiler pass。
- Trace、日志、截图、隐私数据保存策略。
- CI、发布、签名、依赖安全策略。

PR 尺寸建议：

- 文档 PR：不限，但要结构清楚。
- 代码 PR：尽量少于 400 行有效 diff。
- 架构性 PR：先 ADR，再实现。
- `spike/*` 分支不直接合并实验堆栈，必须整理成可审查提交。

## 5. 合并策略

默认使用 squash merge：

- PR 标题作为最终提交标题。
- PR 描述保留关键背景、验证方式和风险。
- 合并后删除源分支。

例外：

- release 分支合并可以保留 merge commit。
- 需要保留详细 bisect 历史的低层 runtime 改动，可以使用 rebase merge。

禁止：

- 直接 push 到 `main`。
- 对共享分支使用普通 `--force`。
- 把模型文件、APK/AAB、私钥、token、截图数据直接提交到仓库。

## 6. Commit 信息

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

## 7. Release 与 Tag

在 Android App 可运行前，不启用正式 release 流程，只使用普通提交。

进入可安装阶段后：

- 版本号使用 SemVer：`v0.1.0`、`v0.1.1`。
- 每个 release tag 对应一个可追溯构建。
- debug/internal artifact 可通过 GitHub Actions artifact 保存。
- 公开 release 必须包含 changelog、权限变化说明和已知风险。

## 8. 冲突处理

推荐流程：

```bash
git fetch origin
git switch feature/my-work
git rebase origin/main
```

冲突解决后：

```bash
git add <resolved-files>
git rebase --continue
git push --force-with-lease
```

只允许对自己的短生命周期分支使用 `--force-with-lease`。不要 force push `main` 或多人共享分支。

## 9. 参考资料

- GitHub protected branches: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches
- GitHub rulesets: https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/about-rulesets
- Pro Git book: https://git-scm.com/book
