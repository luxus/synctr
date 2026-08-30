# Mac menu bar extra (not in this cut)

This worker is Linux. A real menu bar extra is a signed Swift/AppKit (or MenuBarExtra) app. That cannot be compiled or verified here, so this directory is a `synctr status --json` consumer sketch, not a shippable `.app`.

A later extra should:

1. Poll `synctr status --json` (or run it on click). Do not invent a second schema.
2. Show rclone found/missing and each profile's `last_run.ok`.
3. Start a sync with `synctr sync <name>`, not by talking to rclone itself.
4. Leave TUI and `synctr watch` as the interactive/watch UIs.

`status-poll.sh` prints a one-line summary from that JSON. Run it against a built `synctr` to see the same fields a menu extra would read.

```
./contrib/menubar/status-poll.sh
SYNCTR_BIN=/path/to/synctr ./contrib/menubar/status-poll.sh
```

Issue #7 stays open until someone builds the extra on a Mac.
