#!/bin/sh
# Default container check: print every expected shell's version, then run the mounted
# binaries: rosamta and rsmt print their version, everything else is a test executable.
set -eu
for s in ${ROSAMTA_SHELLS}; do
  command -v "${s}" >/dev/null || { echo "missing shell: ${s}" >&2; exit 1; }
  if [ "${s}" = sh ]; then
    echo "sh: $(readlink -f /bin/sh)"
  else
    echo "${s}: $("${s}" --version | head -n 1)"
  fi
done
status=0
for b in /opt/rosamta/bin/*; do
  [ -x "${b}" ] || continue
  case "${b##*/}" in
    rosamta | rsmt) "${b}" --version ;;
    *) echo "-- ${b##*/}" && "${b}" -q || status=1 ;;
  esac
done
exit "${status}"
