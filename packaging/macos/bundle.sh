#!/bin/sh
# Assemble rosamta.app with universal binaries from both macOS release builds.
#   packaging/macos/bundle.sh <version> <out-dir>
set -eu
version=$1
out=$2
root=$(cd "$(dirname "$0")/../.." && pwd)
app="${out}/rosamta.app"
rm -rf "${app}"
mkdir -p "${app}/Contents/MacOS"
for bin in rosamta-gui rosamta; do
  lipo -create -output "${app}/Contents/MacOS/${bin}" \
    "${root}/target/aarch64-apple-darwin/release/${bin}" \
    "${root}/target/x86_64-apple-darwin/release/${bin}"
done
sed "s/@VERSION@/${version}/g" "${root}/packaging/macos/Info.plist" > "${app}/Contents/Info.plist"

# The background service: the CLI again, in an LSUIElement helper app that
# launchd starts through the LaunchAgent (ADR 0004).
agent="${app}/Contents/Library/LoginItems/rosamta-agent.app"
mkdir -p "${agent}/Contents/MacOS" "${app}/Contents/Library/LaunchAgents"
cp "${app}/Contents/MacOS/rosamta" "${agent}/Contents/MacOS/rosamta"
sed "s/@VERSION@/${version}/g" "${root}/packaging/macos/Agent-Info.plist" > "${agent}/Contents/Info.plist"
cp "${root}/packaging/macos/tw.xinshou.rosamta.agent.plist" "${app}/Contents/Library/LaunchAgents/"
for plist in "${app}/Contents/Info.plist" "${agent}/Contents/Info.plist" \
  "${app}/Contents/Library/LaunchAgents/tw.xinshou.rosamta.agent.plist"; do
  plutil -lint "${plist}" >/dev/null
done
