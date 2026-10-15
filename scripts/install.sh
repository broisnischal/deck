#!/bin/sh
# Build the release binary and install it as a macOS app.
#
#     scripts/install.sh            # /Applications/Deck.app + ~/.local/bin/deck
#     DECK_APP=~/Applications/Deck.app DECK_BIN=/usr/local/bin scripts/install.sh
#
# The bundle is assembled in a temporary folder and swapped in at the end, so
# a failed build never leaves a half-installed app behind.
set -eu

cd "$(dirname "$0")/.."
APP="${DECK_APP:-/Applications/Deck.app}"
BIN="${DECK_BIN:-$HOME/.local/bin}"

cargo build --release -p app

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
BUNDLE="$STAGE/Deck.app"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
cp target/release/deck "$BUNDLE/Contents/MacOS/deck"
cp packaging/Info.plist "$BUNDLE/Contents/Info.plist"
cp packaging/deck.icns "$BUNDLE/Contents/Resources/deck.icns"
# Ad-hoc signature: enough for a build you made yourself to launch cleanly.
codesign --force --sign - "$BUNDLE" >/dev/null

# Quit a running copy first; replacing an app under a live process confuses
# Launch Services.
osascript -e 'tell application id "com.broisnischal.deck" to quit' >/dev/null 2>&1 || true
pkill -f "$APP/Contents/MacOS/deck" 2>/dev/null || true

rm -rf "$APP"
mv "$BUNDLE" "$APP"
# Let Finder and Spotlight see the new bundle (and its icon) right away.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$APP" >/dev/null 2>&1 || true

# The CLI, so `deck open URL` works from any terminal, not just deck's own.
mkdir -p "$BIN"
ln -sf "$APP/Contents/MacOS/deck" "$BIN/deck"

echo "Installed $APP"
echo "CLI: $BIN/deck"
