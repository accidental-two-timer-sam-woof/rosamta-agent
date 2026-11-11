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
set -o pipefail
apk add --no-cache -q curl gcc musl-dev >/dev/null
export CARGO_TARGET_DIR=/target PATH="/root/.cargo/bin:${PATH}"
[ -x /root/.cargo/bin/rustup ] || curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain none
rustup toolchain install >/dev/null 2>&1
t="$(uname -m)-unknown-linux-musl"
rustup target add "${t}" >/dev/null 2>&1
rm -rf /out/*
# The target volume outlives a failed build; never copy its old binaries.
rm -f "/target/${t}/debug/rosamta" "/target/${t}/debug/rsmt"
cargo test --quiet --locked --no-run --target "${t}" -p rosamta-core -p rosamta --message-format json \
  | grep -o "\"executable\":\"[^\"]*\"" | cut -d\" -f4 | while read -r exe; do cp "${exe}" /out/; done
[ -n "$(ls /out)" ] || { echo "no test executables were built" >&2; exit 1; }
cargo build --locked --target "${t}" -p rosamta 2>&1 | tail -n 1
cp "/target/${t}/debug/rosamta" "/target/${t}/debug/rsmt" /out/
ls /out
'
