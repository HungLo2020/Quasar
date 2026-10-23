---
name: update-and-push
description: Bumps the Quasar workspace version (third/patch digit by default), verifies the tree, commits and pushes it, waits for every GitHub workflow to succeed, then tags that commit as vX.Y.Z. Use when the user says "update and push", "ship it", "cut a release", "bump the version", "push and tag", or asks to publish the current state.
license: MIT
compatibility: Requires git, cargo, python3, and an authenticated GitHub CLI (gh).
metadata:
  author: Quasar
  version: "1.0"
---

# Update and push

Ship the current tree as a versioned Quasar release. Keep the steps in order:
bump → verify → push → CI green → tag. Tagging last means every `vX.Y.Z` tag
points at a commit whose workflows are known to pass.

## Before you start

Work from the repository root. If the user asks for only part of the workflow
(for example "bump and commit but don't push"), stop at that step.

Version rules:

- Bump the **third** number by default: `0.2.1` → `0.2.2`.
- Bump major or minor only when the user explicitly asks for it.
- If the user names an exact version, use it exactly.
- All crates inherit `[workspace.package].version` from the root `Cargo.toml`.
  Do not edit individual crate manifests.

## Checklist

Copy this checklist into your response and tick items off:

```text
- [ ] 1. Inspect repository and version
- [ ] 2. Bump root workspace version
- [ ] 3. Refresh Cargo.lock
- [ ] 4. run fmt + tests + clippy + verify scripts
- [ ] 5. Commit
- [ ] 6. Push branch
- [ ] 7. Wait for all workflows on the commit
- [ ] 8. Create and push vX.Y.Z tag
- [ ] 9. Wait for tag-triggered workflows
- [ ] 10. Report version, commit, tag, and CI results
```

### 1. Inspect

```bash
git status --short --branch
git remote -v
git fetch --tags origin
git tag --list --sort=-v:refname | head -20
```

- If the working tree has unrelated changes, ask whether to include them.
  Never stash, discard, reset, or rewrite the user's work.
- If the branch is behind `origin`, run `git pull --ff-only` before editing.
- If the target tag already exists, stop and ask rather than moving or reusing it.
- Confirm the branch is the one to release (normally `main`).

### 2. Bump the version

Use the bundled script so the patch bump is deterministic:

```bash
python3 .agents/skills/update-and-push/scripts/bump_version.py
```

For an explicit major/minor bump, pass `major` or `minor`. For an exact version,
pass `--version X.Y.Z`. The script updates the `version` field under
`[workspace.package]` and the checked-in release references (root README, render
crate READMEs, and the Pong runtime note).

### 3. Refresh Cargo.lock

Path packages record their versions in `Cargo.lock`, so update it before using
`--locked`:

```bash
cargo check --workspace --all-targets
git diff -- Cargo.lock
```

The lock diff should contain only the workspace package versions (`pong`,
`tilequest`, `quasar-*`, `vulkanic-gal`). If dependencies changed unexpectedly,
stop and investigate instead of hand-editing the lockfile.

### 4. Verify

Run the same checks CI runs. Do not continue if a command you can run fails.

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 tools/verify_pong.py
python3 tools/verify_tilemap.py
```

The ignored GPU/Vulkan tests need a graphics environment. If it is unavailable
locally, say so and rely on the `gpu-composition` CI job; never claim they ran.
Fix failures, rerun the affected command, and include the fix in the release
commit. Check `git status` afterwards and leave ignored artifacts such as
`target/`, `dist/`, and `data/` out of the commit.

### 5. Commit

```bash
git add -A
git commit -m "Release Quasar X.Y.Z"
git show --stat --oneline HEAD
```

Match the existing release style. Do not amend a commit that is already pushed
unless the user explicitly asks.

### 6. Push

```bash
git push origin HEAD
git rev-parse HEAD
```

Never force-push. Save the printed SHA for the remaining steps.

### 7. Wait for workflows on the commit

```bash
python3 .agents/skills/update-and-push/scripts/wait_for_checks.py
```

The script polls every workflow run for `HEAD` until all complete and exits
non-zero if any run fails or is cancelled. If a run fails:

```bash
gh run view <run-id> --log-failed
```

Fix the cause, commit, push, and repeat from step 4. Tag only when the current
tip commit is green. Treat `skipped` and `neutral` as acceptable; `cancelled`
and `failure` are not.

### 8. Tag the green commit

Existing Quasar tags are lightweight and look like `v0.2.1`:

```bash
git tag "vX.Y.Z"
git push origin "vX.Y.Z"
```

The tag must point at the commit that passed CI. Verify it:

```bash
git rev-parse HEAD
git ls-remote --tags origin "refs/tags/vX.Y.Z"
```

### 9. Wait for tag-triggered workflows

Pushing the tag triggers the `push` workflow again:

```bash
python3 .agents/skills/update-and-push/scripts/wait_for_checks.py
```

If a tag-triggered run fails, report it and ask before deleting or moving the
tag. A published tag should be treated as immutable.

### 10. Report

Give the user:

- old → new version
- commit SHA and commit subject
- tag name and that it was pushed
- workflow names and final conclusions
- any checks that could not run locally and why

## Failure rules

- Never tag before the commit's workflows are green.
- Never reuse, move, or force-push an existing tag.
- Never bump major/minor or invent a version without explicit permission.
- Never discard uncommitted user changes to get a clean release.
- If `gh` is unauthenticated or the remote is unreachable, stop and report
  instead of tagging locally and hoping CI passes later.
