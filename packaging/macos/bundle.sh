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
plutil -lint "${app}/Contents/Info.plist" >/dev/null
