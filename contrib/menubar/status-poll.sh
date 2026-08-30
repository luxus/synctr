#!/bin/sh
# Thin synctr status --json consumer. A macOS menu bar extra would parse the
# same object (rclone.found, profiles[].name, profiles[].last_run.ok).
set -eu
bin="${SYNCTR_BIN:-synctr}"
json="$("$bin" status --json)"
python3 -c '
import json, sys
data = json.load(sys.stdin)
rclone = data.get("rclone") or {}
found = rclone.get("found") is True
profiles = data.get("profiles") or []
ok = fail = never = 0
for p in profiles:
    last = p.get("last_run")
    if last is None:
        never += 1
    elif last.get("ok") is True:
        ok += 1
    else:
        fail += 1
rclone_s = "rclone" if found else "no-rclone"
print(f"{rclone_s} profiles={len(profiles)} ok={ok} fail={fail} never={never}")
' <<EOF
$json
EOF
