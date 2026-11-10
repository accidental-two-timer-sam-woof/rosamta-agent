#!/bin/sh
# Build the test executables and the CLI as static musl binaries, for run.sh to mount.
#   tests/docker/build-tests.sh <out-dir>
# Runs cargo inside Alpine; the named volumes keep the toolchain and target dir between runs.
# The repo is mounted at its host path, where tests that read fixtures look for it.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "${here}/../.." && pwd)
out=${1:?usage: build-tests.sh <out-dir>}
mkdir -p "${out}"
out=$(cd "${out}" && pwd)
export DOCKER_DEFAULT_PLATFORM="${DOCKER_DEFAULT_PLATFORM:-linux/amd64}"
arch=${DOCKER_DEFAULT_PLATFORM##*/}

docker run --rm \
  -v "${root}:${root}:ro" -w "${root}" -v "${out}:/out" \
  -v "rosamta-musl-home-${arch}:/root" -v "rosamta-musl-target-${arch}:/target" \
  alpine:latest sh -euc '
apk add --no-cache -q curl gcc musl-dev >/dev/null
export CARGO_TARGET_DIR=/target PATH="/root/.cargo/bin:${PATH}"
[ -x /root/.cargo/bin/rustup ] || curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain none
rustup toolchain install >/dev/null 2>&1
t="$(uname -m)-unknown-linux-musl"
rustup target add "${t}" >/dev/null 2>&1
rm -rf /out/*
cargo test --locked --no-run --target "${t}" -p rosamta-core -p rosamta --message-format json 2>/dev/null \
  | grep -o "\"executable\":\"[^\"]*\"" | cut -d\" -f4 | while read -r exe; do cp "${exe}" /out/; done
cargo build --locked --target "${t}" -p rosamta 2>&1 | tail -n 1
cp "/target/${t}/debug/rosamta" "/target/${t}/debug/rsmt" /out/
ls /out
'
