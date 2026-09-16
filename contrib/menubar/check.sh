#!/bin/sh
# Linux-side checks for the macOS extra: package layout, JSON contract,
# status-poll.sh against fixtures, and against a built synctr when present.
set -eu
root="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
app="$root/SynctrMenuBar"
src="$app/Sources/SynctrMenuBar"
fail=0

say() { printf '%s\n' "$*"; }
die() { say "FAIL: $*"; fail=1; }
ok() { say "ok: $*"; }

need() {
  if [ ! -f "$1" ]; then
    die "missing $1"
  else
    ok "have ${1#"$root"/}"
  fi
}

need "$app/Package.swift"
need "$app/Info.plist"
need "$app/SynctrMenuBar.entitlements"
need "$app/SynctrMenuBar.xcodeproj/project.pbxproj"
need "$app/SynctrMenuBar.xcodeproj/xcshareddata/xcschemes/SynctrMenuBar.xcscheme"
need "$app/build-macos.sh"
need "$src/SynctrMenuBarApp.swift"
need "$src/AppModel.swift"
need "$src/MenuBarView.swift"
need "$src/StatusSnapshot.swift"
need "$src/SynctrCLI.swift"
need "$root/status-poll.sh"
need "$root/fixtures/synctr-stub.sh"
need "$root/fixtures/status-ok.json"
need "$root/fixtures/status-missing-rclone.json"
need "$root/fixtures/status-progress.json"

if grep -q 'MenuBarExtra' "$src/SynctrMenuBarApp.swift"; then
  ok "MenuBarExtra in SynctrMenuBarApp.swift"
else
  die "SynctrMenuBarApp.swift has no MenuBarExtra"
fi

if grep -q 'synctr sync' "$src/SynctrCLI.swift" && grep -q 'status", "--json' "$src/SynctrCLI.swift"; then
  ok "CLI wrapper calls status --json and sync"
else
  die "SynctrCLI.swift does not invoke status --json / sync"
fi

if grep -q 'LSUIElement' "$app/Info.plist"; then
  ok "Info.plist is a menu extra (LSUIElement)"
else
  die "Info.plist missing LSUIElement"
fi

if grep -q 'D2XV456V9A' "$app/SynctrMenuBar.xcodeproj/project.pbxproj"; then
  ok "Xcode project has DEVELOPMENT_TEAM"
else
  die "project.pbxproj missing team id"
fi

if grep -q 'macOS(.v13)' "$app/Package.swift" && grep -q 'executableTarget' "$app/Package.swift"; then
  ok "Package.swift is a macOS 13 executable"
else
  die "Package.swift layout unexpected"
fi

python3 - "$root" <<'PY' || die "JSON contract / Swift coding keys"
import json, pathlib, sys

root = pathlib.Path(sys.argv[1])
swift = (root / "SynctrMenuBar/Sources/SynctrMenuBar/StatusSnapshot.swift").read_text()

required_keys = [
    '"found"',
    '"path"',
    '"source"',
    '"detail"',
    '"finished_at_unix"',
    '"finished_at"',
    '"exit_code"',
    '"ok"',
    '"extra_flags"',
    '"extra_ignore"',
    '"last_run"',
    '"total_bytes"',
    '"speed_bps"',
    '"eta_secs"',
    '"total_transfers"',
    '"dry_run"',
    '"updated_at_unix"',
    '"updated_at"',
]
missing = [k for k in required_keys if k not in swift]
if missing:
    print("FAIL: StatusSnapshot.swift missing coding keys:", ", ".join(missing))
    sys.exit(1)
print("ok: StatusSnapshot.swift coding keys match the status --json contract")

def check_contract(path: pathlib.Path) -> None:
    v = json.loads(path.read_text())
    rclone = v["rclone"]
    assert isinstance(rclone.get("found"), bool), f"{path}: rclone.found"
    profiles = v["profiles"]
    assert isinstance(profiles, list), f"{path}: profiles"
    for p in profiles:
        for key in ("name", "local", "remote", "mode", "extra_flags", "extra_ignore"):
            assert key in p, f"{path}: missing profile.{key}"
        last = p.get("last_run")
        if last is not None:
            for key in ("finished_at_unix", "finished_at", "exit_code", "ok"):
                assert key in last, f"{path}: missing last_run.{key}"
        progress = p.get("progress")
        if progress is not None:
            for key in (
                "bytes",
                "total_bytes",
                "transfers",
                "total_transfers",
                "dry_run",
                "updated_at_unix",
                "updated_at",
            ):
                assert key in progress, f"{path}: missing progress.{key}"

for name in (
    "status-ok.json",
    "status-missing-rclone.json",
    "status-progress.json",
):
    check_contract(root / "fixtures" / name)
    print(f"ok: fixture {name} matches status --json contract")
PY

chmod +x "$root/status-poll.sh" "$root/fixtures/synctr-stub.sh" "$app/build-macos.sh"

expect_poll() {
  fixture="$1"
  want="$2"
  got="$(SYNCTR_BIN="$root/fixtures/synctr-stub.sh" SYNCTR_STATUS_FIXTURE="$fixture" "$root/status-poll.sh")"
  if [ "$got" = "$want" ]; then
    ok "status-poll.sh $(basename "$fixture") -> $got"
  else
    die "status-poll.sh $(basename "$fixture"): got [$got] want [$want]"
  fi
}

expect_poll "$root/fixtures/status-ok.json" "rclone profiles=1 ok=1 fail=0 never=0 running=0"
expect_poll "$root/fixtures/status-missing-rclone.json" "no-rclone profiles=1 ok=0 fail=0 never=1 running=0"
expect_poll "$root/fixtures/status-progress.json" "rclone profiles=2 ok=0 fail=1 never=1 running=1"

synctr_bin=""
if [ -n "${SYNCTR_BIN:-}" ] && [ -x "${SYNCTR_BIN}" ]; then
  synctr_bin="$SYNCTR_BIN"
elif [ -x "$root/../../target/debug/synctr" ]; then
  synctr_bin="$root/../../target/debug/synctr"
elif [ -x "$root/../../target/release/synctr" ]; then
  synctr_bin="$root/../../target/release/synctr"
elif command -v synctr >/dev/null 2>&1; then
  synctr_bin="$(command -v synctr)"
fi

if [ -n "$synctr_bin" ]; then
  home="$(mktemp -d)"
  trap 'rm -rf "$home"' EXIT
  mkdir -p "$home/docs" "$home/.config" "$home/.local/state" "$home/.cache"
  isolated() {
    HOME="$home" \
    XDG_CONFIG_HOME="$home/.config" \
    XDG_STATE_HOME="$home/.local/state" \
    XDG_CACHE_HOME="$home/.cache" \
    env -u SYNCTR_RCLONE \
    "$@"
  }
  isolated "$synctr_bin" profile add docs \
    --local "$home/docs" \
    --remote b2:bucket/docs \
    --mode sync >/dev/null
  got="$(isolated env SYNCTR_BIN="$synctr_bin" "$root/status-poll.sh")"
  if [ "$got" = "no-rclone profiles=1 ok=0 fail=0 never=1 running=0" ] || echo "$got" | grep -Eq '^rclone profiles=1 ok=0 fail=0 never=1 running=0$'; then
    ok "status-poll.sh against $synctr_bin with a profile -> $got"
  else
    die "live status-poll.sh with profile: [$got]"
  fi
else
  say "skip: no built synctr (set SYNCTR_BIN or cargo build -p synctr)"
fi

if [ "$fail" -ne 0 ]; then
  say "contrib/menubar/check.sh failed"
  exit 1
fi
say "contrib/menubar/check.sh passed"
exit 0
