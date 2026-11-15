# Worktrees

Where agents commit and merge, and how to run several agents on separate branches at the same time.

- **The main checkout belongs to the user** and can switch branches mid-session, so the branch from a session's start may no longer be checked out. Commit and merge agent work in a worktree, docs included, and push with `git push origin HEAD:<branch>`.
- **One worktree per branch**, outside the repo: `git worktree add -b <branch> ../sett-wt/<name> <base>`. Use these for every subagent, research included, rather than the Agent tool's own isolation: its worktrees under `.claude/worktrees/` refuse `git config` and many compound commands.
- **The pre-commit hook is already on** in every worktree: `core.hooksPath` lives in the shared `.git/config`.
- **Screenshots come from the main session.** A subagent can't capture (in Claude Code the sandbox exception doesn't reach it), so a ticket's screenshots are the main session's check, not an implementer's acceptance item. When building a spec, screenshot each UI ticket right after merging it, before the next ticket starts. Capture a worktree's build without switching the main checkout: `scripts/screenshot --root ../sett-wt/<name> --target ../sett-wt/.target-<name> …`.
- **Give each worktree its own `CARGO_TARGET_DIR`**, for example `../sett-wt/.target-<name>`. With one shared target dir across worktrees, Cargo once linked another worktree's `sett-store` (same package name and version, different path), and the tests ran against the wrong code.
- **Budget disk space.** Each target dir grows to 6–19 GB, because the GPUI dependency tree is large.
- **Implementer branches stay local.** Merge them into the integration branch locally, and push only the integration branch. Deleting remote branches is blocked for agents.
- **Clean up** once the branch is merged: `git worktree remove ../sett-wt/<name>`, `git branch -d <branch>`, then delete its target dir. `git branch -d` checks against the checked-out branch, so it refuses a branch merged only into the integration branch; confirm with `git merge-base --is-ancestor <branch> <integration>`, then use `git branch -D`.
- **sccache** shares compiled dependencies across target dirs, which cuts both build time and disk use. On the driving dev's machine it is already on for every cargo build, through `build.rustc-wrapper` in `~/.cargo/config.toml`, so worktrees need nothing extra. It is set up per user, not in the repo, so CI and machines without sccache still build.
