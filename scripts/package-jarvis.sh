#!/bin/bash
set -e

cargo build --release --manifest-path jarvis/Cargo.toml

VERSION="0.1.0"
DIST="dist/jarvis-v${VERSION}"

rm -rf "$DIST"
mkdir -p "$DIST"

cp jarvis/target/release/jarvis "$DIST/"
cp -r services/vexa-bot "$DIST/vexa-bot"

echo "Packaged to $DIST"
echo "Run: cd $DIST && ./jarvis --openai-key sk-... --meet https://meet.google.com/..."
