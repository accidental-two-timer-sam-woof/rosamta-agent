#!/bin/sh
# Build the test images and run a check in each container.
#   tests/docker/run.sh [image...]      images are the *.Dockerfile names; default: all
#   ROSAMTA_BIN_DIR   Linux binaries to mount read-only at /opt/rosamta/bin
#   ROSAMTA_TEST_CMD  command to run instead of check.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "${here}/../.." && pwd)
# Same architecture as the CI runners, so one x86_64 musl build serves every image.
# Set DOCKER_DEFAULT_PLATFORM=linux/arm64 to run natively on Apple Silicon.
export DOCKER_DEFAULT_PLATFORM="${DOCKER_DEFAULT_PLATFORM:-linux/amd64}"

if [ $# -eq 0 ]; then
  for f in "${here}"/*.Dockerfile; do
    n=${f##*/}
    set -- "$@" "${n%.Dockerfile}"
  done
fi

bin_dir=
if [ -n "${ROSAMTA_BIN_DIR:-}" ]; then
  bin_dir=$(cd "${ROSAMTA_BIN_DIR}" && pwd)
fi

status=0
for name in "$@"; do
  echo "== ${name}"
  if ! docker build -q -t "rosamta-test:${name}" -f "${here}/${name}.Dockerfile" "${here}" >/dev/null; then
    echo "image build failed: ${name}" >&2
    status=1
    continue
  fi
  # The repo sits at its host path too, for tests that read fixtures from the source tree.
  docker run --rm -v "${here}:/opt/rosamta/tests:ro" -v "${root}:${root}:ro" \
    ${bin_dir:+-v "${bin_dir}:/opt/rosamta/bin:ro"} "rosamta-test:${name}" \
    sh -c "${ROSAMTA_TEST_CMD:-sh /opt/rosamta/tests/check.sh}" || status=1
done
exit "${status}"
