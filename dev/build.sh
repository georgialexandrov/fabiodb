#!/bin/sh
# Release build of the app and the MCP server for this Mac.
#
# Signs with the "Fabio Local" certificate when it's in the keychain, so macOS
# ties Keychain "Always Allow" to the certificate instead of to one build:
# answered once, it holds across rebuilds. Without it, builds are ad-hoc (as
# CI's are) and every new build asks again. To create the certificate:
# Keychain Access → Certificate Assistant → Create a Certificate…, name
# "Fabio Local", Self-Signed Root, Code Signing.
set -e
cd "$(dirname "$0")/.."

if security find-identity -p codesigning 2>/dev/null | grep -q '"Fabio Local"'; then
  export APPLE_SIGNING_IDENTITY="Fabio Local"
fi

(cd app && pnpm tauri build --bundles app)
cargo build -q -p fabiodb-mcp --release
if [ -n "$APPLE_SIGNING_IDENTITY" ]; then
  codesign -f -s "$APPLE_SIGNING_IDENTITY" --identifier dev.alexandrov.fabio.mcp target/release/fabiodb-mcp
fi

codesign -dv target/release/bundle/macos/Fabio.app 2>&1 | grep -E '^(Authority|Signature)=' || true
echo "target/release/bundle/macos/Fabio.app"
echo "target/release/fabiodb-mcp"
