#!/bin/sh
set -eu
if [ -n "${SYNCTR_STUB_LOG:-}" ]; then
  {
    printf '%s' "$0"
    for a in "$@"; do
      printf '\t%s' "$a"
    done
    printf '\n'
  } >> "$SYNCTR_STUB_LOG"
fi
exit "${SYNCTR_STUB_EXIT:-0}"
