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
if [ -n "${SYNCTR_STUB_STDERR:-}" ]; then
  printf '%s\n' "$SYNCTR_STUB_STDERR" >&2
fi
if [ -n "${SYNCTR_STUB_SLEEP:-}" ]; then
  sleep "$SYNCTR_STUB_SLEEP"
fi
exit "${SYNCTR_STUB_EXIT:-0}"
