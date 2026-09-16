# 0.1.0 feature-complete audit (follow-up)

Canonical claim table and issues **#11–#16**: [`docs/0.1.0-audit.md`](docs/0.1.0-audit.md). That file is the source of truth. This note does not replace it.

Verdict (same): **no** — not feature-complete as documented. Mac menu bar remains #7.

**P0 is [#11](https://github.com/luxus/synctr/issues/11)** (`rust-version` 1.83 cannot `cargo test --workspace --locked`). #19 was closed as a duplicate of #11.

## This PR

- Product: `status --json` maps a missing rclone override (`RcloneOverrideMissing`) to `rclone.found: false` and exit 0 — the override case of #13 / #22.
- Watch ignored-directory wakes: **not here.** [#27](https://github.com/luxus/synctr/pull/27) is the stronger fix (directory roots, symlink/canonical paths, metadata events, CLI test). #21 is that directory-root bug.

## Extra issues from this pass

Not a second ranking of #11–#16. #12 stays the release-assets P1; #13 stays the broader status fail-closed P1.

| Pri | Issue | Note |
| --- | --- | --- |
| P1 | #20 | Duplicate of canonical #12 |
| P1 | #21 | Watch ignored directory roots — covered by #27, not this PR |
| P1 | #22 | Override-missing JSON — **fixed here**; subset of #13 |
| P2 | #23 | TUI `d` / stop vs `last_run.ok` |
| P2 | #24 | MIT in Cargo.toml, no LICENSE |
| P2 | #25 | no `flake.lock` |
| later | #7 | Mac menu bar (out of 0.1.0) |

## `cargo test --workspace --locked`

Same as `docs/0.1.0-audit.md`: **fail** on rustc 1.83 (#11); **pass** on rustc 1.98.1.
