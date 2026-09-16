#!/bin/sh
# Build SynctrMenuBar.app on macOS. Does not run on Linux.
set -eu
cd "$(dirname "$0")"
DEST="${1:-./dist}"
mkdir -p "$DEST"
xcodebuild \
  -project SynctrMenuBar.xcodeproj \
  -scheme SynctrMenuBar \
  -configuration Release \
  -derivedDataPath ./DerivedData \
  CODE_SIGNING_ALLOWED=NO
APP="$(find ./DerivedData/Build/Products/Release -maxdepth 1 -name 'SynctrMenuBar.app' | head -n 1)"
if [ -z "$APP" ]; then
  echo "xcodebuild did not produce SynctrMenuBar.app" >&2
  exit 1
fi
rm -rf "$DEST/SynctrMenuBar.app"
cp -R "$APP" "$DEST/SynctrMenuBar.app"
echo "built $DEST/SynctrMenuBar.app"
echo "open $DEST/SynctrMenuBar.app"
