#!/bin/sh
# Build the test images and run a check in each container.
#   tests/docker/run.sh [image...]      images are the *.Dockerfile names; default: all
#   ROSAMTA_BIN_DIR   Linux binaries to mount read-only at /opt/rosamta/bin
#   ROSAMTA_TEST_CMD  command to run instead of check.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
# Same architecture as the CI runners, so one x86_64 musl build serves every image.
export DOCKER_DEFAULT_PLATFORM=linux/amd64

if [ $# -eq 0 ]; then
  for f in "${here}"/*.Dockerfile; do
    n=${f##*/}
    set -- "$@" "${n%.Dockerfile}"
  done
fi

bin_mount=
if [ -n "${ROSAMTA_BIN_DIR:-}" ]; then
  bin_mount="-v $(cd "${ROSAMTA_BIN_DIR}" && pwd):/opt/rosamta/bin:ro"
fi

status=0
for name in "$@"; do
  echo "== ${name}"
  if ! docker build -q -t "rosamta-test:${name}" -f "${here}/${name}.Dockerfile" "${here}" >/dev/null; then
    echo "image build failed: ${name}" >&2
    status=1
    continue
  fi
  # shellcheck disable=SC2086 # bin_mount is either empty or two words
  docker run --rm -v "${here}:/opt/rosamta/tests:ro" ${bin_mount} "rosamta-test:${name}" \
    sh -c "${ROSAMTA_TEST_CMD:-sh /opt/rosamta/tests/check.sh}" || status=1
done
exit "${status}"
