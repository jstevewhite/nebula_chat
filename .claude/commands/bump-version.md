# Bump Nebula Version

Bump the version to `$ARGUMENTS` across all four locations, regenerate the Cargo lockfile, sync the Tauri npm packages with the Rust crate version, commit, tag, and push. Pushing the tag triggers the `release` GitHub Actions workflow.

## Normalize the argument FIRST

The user may type `$ARGUMENTS` as either `0.7.1` or `v0.7.1`. Before doing anything else, derive these two forms and use them consistently below — do NOT just string-concat `v$ARGUMENTS`, because that produces `vv0.7.1` when the user already included the leading `v`.

- **`VERSION`** = the bare semver (`0.7.1`) — strip a leading `v` if present. Use this in `Cargo.toml`, `tauri.conf.json`, `package.json`, and the commit message.
- **`TAG`** = `v` + `VERSION` (`v0.7.1`) — exactly one `v`. Use this in the `README.md` "Current version" line and as the git tag.

If you can't tell which form you have, default to stripping any leading `v` and re-prefixing once.

For beta builds, you typically do NOT need to run this command — the `beta` workflow is `workflow_dispatch` and you trigger it manually from the Actions tab, picking any tag label you want. Use this command only when cutting a real release (or when you want a beta tag committed to history).

## Files to update

| File | Field |
|------|-------|
| `src-tauri/Cargo.toml` | `version = "X.Y.Z"` (line 3, under `[package]`) |
| `src-tauri/tauri.conf.json` | `"version": "X.Y.Z"` (top-level) |
| `package.json` | `"version": "X.Y.Z"` |
| `README.md` | `**Current version:** \`vX.Y.Z\`` (around line 9) |

**Do NOT touch** the `### What's new in vX.Y.Z` header in `README.md` — that line describes the contents of the previous release. Update it manually when you write the changelog entry for the new version.

## Steps

1. Edit all four files, replacing the old version. Use **`VERSION`** (bare, no `v`) in `Cargo.toml`, `tauri.conf.json`, and `package.json`. Use **`TAG`** (with one `v`) in `README.md`'s "Current version" line.

2. Regenerate the Cargo lockfile:
   ```
   cd src-tauri && ~/.cargo/bin/cargo generate-lockfile
   ```

3. **Sync the Tauri npm packages with the Rust `tauri` crate version.** Cargo's `generate-lockfile` quietly pulls in the latest 2.x of the `tauri` crate; if the npm `@tauri-apps/api` and `@tauri-apps/cli` lag a minor version, the `tauri build` script aborts with a "version mismatched Tauri packages" error and CI fails. To avoid that:

   ```
   # Read the resolved tauri crate minor (e.g. 2.11) from Cargo.lock.
   # NOTE: avoid awk's positional fields ($N) here — Claude Code's slash-command
   # argument processor expands $1/$2/$3 into the empty string, which silently
   # corrupts the version into "version = 2.11" and breaks the npm install.
   # Use grep + cut, which has no positional-field syntax.
   tauri_minor=$(grep -A 1 '^name = "tauri"$' src-tauri/Cargo.lock | grep -m1 '^version' | cut -d'"' -f2 | cut -d. -f1-2)

   # Bump both npm packages to the same minor (caret range)
   npm install "@tauri-apps/api@^${tauri_minor}" "@tauri-apps/cli@^${tauri_minor}"
   ```

   Confirm the resolved versions match by spot-checking `package-lock.json` for `node_modules/@tauri-apps/api` and `node_modules/@tauri-apps/cli` — both should be at the same minor as the Rust `tauri` crate.

4. Verify the build still passes locally before tagging:
   ```
   cd src-tauri && cargo build && cd ..
   npm run build
   ```

5. Stage and commit (use **`TAG`** in the commit message so the leading `v` is always present):
   ```
   git add src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/Cargo.lock \
           package.json package-lock.json README.md
   git commit -m "chore: bump version to <TAG>"   # e.g. "chore: bump version to v0.7.1"
   ```

6. Tag and push — the tag is exactly **`TAG`** (one `v`):
   ```
   git tag <TAG>          # e.g. git tag v0.7.1   — NEVER git tag vv0.7.1
   git push
   git push origin <TAG>
   ```

   The tag push fires `.github/workflows/release.yml`, which builds bundles on every platform and uploads them to a draft GitHub release, then flips the release out of draft once all platforms succeed.

## If the build fails on a Tauri version mismatch

Check the CI error for the actual versions, then on `master`:

```
npm install @tauri-apps/api@^<MINOR> @tauri-apps/cli@^<MINOR>
git add package.json package-lock.json
git commit -m "fix(build): sync @tauri-apps npm packages to <MINOR>"
git push
```

Then bump to the next patch (e.g. 0.5.2 → 0.5.3) and re-tag — don't move existing tags. CI will rebuild on the new tag.

## First-time note: stale Cargo.toml version

If `src-tauri/Cargo.toml` is still at `0.1.0` from the project scaffold, the first bump after adopting this command will jump that file by several minor versions. That's expected — from then on it stays in sync.
