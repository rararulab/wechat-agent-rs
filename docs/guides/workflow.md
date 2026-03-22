# Development Workflow — Issue → Worktree → PR → Merge

**Every code change — no matter how small — MUST follow this workflow.** There are zero exceptions: single-line fixes, typo corrections, config tweaks, doc updates, and refactors all go through issue + worktree + PR.

```
1. CREATE ISSUE    →  gh issue create + labels
2. CREATE WORKTREE →  git worktree add .worktrees/issue-{N}-{name} -b issue-{N}-{name}
3. WORK            →  All edits happen inside the worktree
4. VERIFY          →  cargo check / cargo test
5. PUSH & PR       →  git push -u origin + gh pr create
6. CI GREEN        →  gh pr checks --watch (MANDATORY before reporting done)
7. CLEANUP         →  git worktree remove + git branch -d (after PR merged)
```

## Step 1: Create Issue

```bash
gh issue create \
  --title "<type>(<scope>): <description>" \
  --label "agent:claude" \
  --body "<context and acceptance criteria>"
```

**Labels** (all issues MUST have labels):
- **Agent**: `agent:claude`
- **Type**: `bug`, `enhancement`, `refactor`, `chore`

## Step 2: Create Worktree
```bash
git worktree add .worktrees/issue-{N}-{short-name} -b issue-{N}-{short-name}
```

## Step 3: Work in Worktree
- All code edits happen exclusively inside the worktree directory
- Independent issues can be dispatched in parallel (each in its own worktree)

## Step 4: Verify
```bash
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Step 5: Push & Create PR
```bash
git push -u origin issue-{N}-{short-name}
gh pr create --title "<type>(<scope>): <description> (#N)" \
  --body "Closes #N" \
  --label "<type-label>"
```

## Step 6: Wait for CI Green (MANDATORY)
```bash
gh pr checks {PR-number} --watch
```
Do NOT report PR as complete while CI is pending or failing.

## Step 7: Cleanup (after PR merged)
```bash
git worktree remove .worktrees/issue-{N}-{short-name}
git branch -d issue-{N}-{short-name}
```
