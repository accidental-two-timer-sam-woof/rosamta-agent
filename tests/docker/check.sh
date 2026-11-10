#!/bin/sh
# Default container check: print every expected shell's version, then run any mounted binaries.
set -eu
for s in ${ROSAMTA_SHELLS}; do
  command -v "${s}" >/dev/null || { echo "missing shell: ${s}" >&2; exit 1; }
  if [ "${s}" = sh ]; then
    echo "sh: $(readlink -f /bin/sh)"
  else
    echo "${s}: $("${s}" --version | head -n 1)"
  fi
done
for b in /opt/rosamta/bin/*; do
  [ -x "${b}" ] && "${b}" --version
done
exit 0
