#!/bin/sh
# Fake synctr for Linux checks. Prints a status --json fixture.
set -eu
root="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
fixture="${SYNCTR_STATUS_FIXTURE:-$root/status-ok.json}"
if [ "${1:-}" = "status" ] && [ "${2:-}" = "--json" ]; then
  cat "$fixture"
  echo
  exit 0
fi
echo "stub only implements status --json" >&2
exit 2
