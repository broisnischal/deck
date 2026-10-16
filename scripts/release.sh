#!/bin/sh
# Build a release of Deck that other Macs will open: dist/Deck-<version>.dmg
# for the website and dist/Deck-<version>.zip for the in-app updater.
#
#     scripts/release.sh             # build, sign, and notarize when set up
#     scripts/release.sh --publish   # then upload both as GitHub release v<version>
#
# macOS only opens a downloaded app that is signed with a Developer ID
# certificate and notarized by Apple. Setting that up is once per Mac and
# needs the Apple Developer Program:
#
#   1. Xcode › Settings › Accounts › Manage Certificates › + › Developer ID Application
#   2. xcrun notarytool store-credentials deck-notary \
#          --apple-id YOU@EXAMPLE.COM --team-id TEAMID --password APP-SPECIFIC-PASSWORD
#      (the password from account.apple.com › Sign-In and Security › App-Specific Passwords)
#
# Without them the build is still made, signed ad hoc for this Mac only, and
# --publish refuses. DECK_SIGN_ID and DECK_NOTARY_PROFILE pick other ones;
# DECK_ARCHS="aarch64" builds for Apple silicon alone (quicker, for trying it).
# A release signs in to https://deck.stroke.click (free trial, then a plan);
# DECK_API=other-url points it elsewhere, DECK_API= makes one without accounts.
set -eu

cd "$(dirname "$0")/.."
PUBLISH=0
[ "${1:-}" = "--publish" ] && PUBLISH=1

VERSION="$(cargo metadata --no-deps --format-version 1 |
    python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "app"))')"
TAG="v$VERSION"
DIST="dist"
APP="$DIST/stage/Deck.app"
DMG="$DIST/Deck-$VERSION.dmg"
ZIP="$DIST/Deck-$VERSION.zip"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# -- what we can sign with -------------------------------------------------------------

SIGN_ID="${DECK_SIGN_ID:-$(security find-identity -v -p codesigning 2>/dev/null |
    sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)}"
PROFILE="${DECK_NOTARY_PROFILE:-deck-notary}"
NOTARIZE=0
if [ -n "$SIGN_ID" ] && xcrun notarytool history --keychain-profile "$PROFILE" >/dev/null 2>&1; then
    NOTARIZE=1
fi

if [ "$PUBLISH" = 1 ]; then
    [ -n "$SIGN_ID" ] || die "no Developer ID Application certificate: other Macs would refuse this build (see the top of this script)"
    [ "$NOTARIZE" = 1 ] || die "no notarytool profile '$PROFILE': other Macs would refuse this build (see the top of this script)"
    [ -z "$(git status --porcelain)" ] || die "commit your changes first: the release is built from the tagged commit"
    git fetch -q origin
    [ "$(git rev-parse HEAD)" = "$(git rev-parse '@{u}')" ] || die "push first: GitHub tags the commit it has"
    gh release view "$TAG" >/dev/null 2>&1 && die "$TAG is already released; raise the version in Cargo.toml"
fi

# -- build: one binary for Apple silicon and Intel --------------------------------------

# Releases sign in to the account server, which runs the trial and the plan.
# DECK_API= (empty) builds one that asks for no account.
export DECK_API="${DECK_API-https://deck.stroke.click}"

say "Building Deck $VERSION${DECK_API:+ (accounts: $DECK_API)}"
bins=""
for arch in ${DECK_ARCHS:-aarch64 x86_64}; do
    cargo build --release -p app --target "$arch-apple-darwin"
    bins="$bins target/$arch-apple-darwin/release/deck"
done

rm -rf "$DIST/stage" "$DMG" "$ZIP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
# shellcheck disable=SC2086
lipo -create $bins -output "$APP/Contents/MacOS/deck"
cp packaging/Info.plist "$APP/Contents/Info.plist"
cp packaging/deck.icns "$APP/Contents/Resources/deck.icns"
/usr/libexec/PlistBuddy \
    -c "Set :CFBundleShortVersionString $VERSION" \
    -c "Set :CFBundleVersion $(git rev-list --count HEAD)" \
    "$APP/Contents/Info.plist"

# -- sign and notarize ------------------------------------------------------------------

notarize() {
    say "Notarizing $(basename "$1") (a few minutes)"
    out="$(xcrun notarytool submit "$1" --keychain-profile "$PROFILE" --wait 2>&1)" || true
    echo "$out" | grep -E "id:|status:" | sed 's/^ */    /' | head -3
    if ! echo "$out" | grep -q "status: Accepted"; then
        id="$(echo "$out" | sed -n 's/^ *id: //p' | head -1)"
        [ -n "$id" ] && xcrun notarytool log "$id" --keychain-profile "$PROFILE" >&2
        die "Apple did not accept $(basename "$1")"
    fi
}

if [ -n "$SIGN_ID" ]; then
    say "Signing as $SIGN_ID"
    # The hardened runtime is what notarization asks for.
    codesign --force --options runtime --timestamp --sign "$SIGN_ID" "$APP"
else
    say "No Developer ID certificate: signing ad hoc, for this Mac only"
    codesign --force --sign - "$APP"
fi
codesign --verify --strict "$APP"

if [ "$NOTARIZE" = 1 ]; then
    # The app first, so the copy in the zip and the disk image carries its
    # own ticket and opens offline too.
    ditto -c -k --keepParent "$APP" "$DIST/notarize.zip"
    notarize "$DIST/notarize.zip"
    rm "$DIST/notarize.zip"
    xcrun stapler staple -q "$APP"
fi

# -- package ----------------------------------------------------------------------------

say "Packaging"
ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP"

# A disk image with Deck and a shortcut to Applications to drag it onto.
ln -s /Applications "$DIST/stage/Applications"
hdiutil create -quiet -fs HFS+ -format UDZO -volname "Deck" -srcfolder "$DIST/stage" "$DMG"
rm "$DIST/stage/Applications"
if [ -n "$SIGN_ID" ]; then
    codesign --force --timestamp --sign "$SIGN_ID" "$DMG"
fi
if [ "$NOTARIZE" = 1 ]; then
    notarize "$DMG"
    xcrun stapler staple -q "$DMG"
    spctl --assess --type open --context context:primary-signature "$DMG"
    spctl --assess --type execute "$APP"
fi

(cd "$DIST" && shasum -a 256 "$(basename "$DMG")" "$(basename "$ZIP")" > "SHA256SUMS-$VERSION")
ls -lh "$DMG" "$ZIP" | awk '{print "    " $5 "  " $9}'

if [ "$NOTARIZE" != 1 ]; then
    echo "Not notarized: fine on this Mac, refused on others. See the top of this script."
fi

# -- publish ----------------------------------------------------------------------------

if [ "$PUBLISH" = 1 ]; then
    say "Publishing $TAG"
    gh release create "$TAG" "$DMG" "$ZIP" "$DIST/SHA256SUMS-$VERSION" \
        --target "$(git rev-parse HEAD)" \
        --title "Deck $VERSION" \
        --notes "What's new: https://deck.stroke.click/changelog/"
    echo "The website's download button and the in-app updater pick it up within five minutes."
fi
