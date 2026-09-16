# Mac menu bar extra

SwiftUI `MenuBarExtra` that drives the `synctr` CLI. It is not a second
sync engine and not a File Provider (#45).

This worker is Linux, so the `.app` is not compiled here. Source under
`SynctrMenuBar/` is the extra. `status-poll.sh` is the same JSON consumer
and **does** run on Linux against a built `synctr`.

## What it does

1. Polls `synctr status --json` (10s idle, 2s while a profile has `progress`).
2. Shows rclone found/missing and each profile's `last_run.ok` (ok / fail / never).
3. **Sync** runs `synctr sync <name>`. **Stop** SIGTERMs the process group that
   command started (rclone is a child of that `synctr`). If the extra did not
   start the job, Stop SIGTERMs `progress.pid` (rclone) from the status JSON.
4. Leaves TUI and `synctr watch` as the interactive/watch UIs.

Put `synctr` on PATH (Homebrew, cargo, nix-darwin) or use **Choose synctr…**.
GUI sessions often have a short PATH; the extra also searches
`~/.cargo/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, and the same nix
locations the CLI uses for rclone.

## Build and run on macOS (Emily / Xcode)

Needs macOS 13+, Xcode 15+, and a working `synctr` on PATH.

One verification command (unsigned local `.app`):

```
xcodebuild -project contrib/menubar/SynctrMenuBar/SynctrMenuBar.xcodeproj -scheme SynctrMenuBar -configuration Release CODE_SIGNING_ALLOWED=NO
```

Then:

```
open contrib/menubar/SynctrMenuBar/DerivedData/Build/Products/Release/SynctrMenuBar.app
```

(`xcodebuild` without `-derivedDataPath` puts the product under Xcode's
default DerivedData. To pin the output path, use `SynctrMenuBar/build-macos.sh`.)

### Xcode GUI

1. `open contrib/menubar/SynctrMenuBar/SynctrMenuBar.xcodeproj`
2. Signing: Automatic, Team **D2XV456V9A** (Personal). App Sandbox is off so
   the extra can exec `synctr`. Notarization / login items are #45, not this.
3. Scheme **SynctrMenuBar**, Run. Look in the menu bar (no Dock icon:
   `LSUIElement`).

`Package.swift` is the same sources for layout review. Opening it in Xcode
builds a CLI executable, not a bundled extra. Use the `.xcodeproj` for the
`.app`.

## Linux consumer check

```
./contrib/menubar/check.sh
cargo +1.88 build -p synctr --locked
SYNCTR_BIN=./target/debug/synctr ./contrib/menubar/status-poll.sh
```

`status-poll.sh` prints one line from the same fields the extra reads:

```
rclone profiles=1 ok=1 fail=0 never=0 running=0
```

Issue #7 / LUX-6.
