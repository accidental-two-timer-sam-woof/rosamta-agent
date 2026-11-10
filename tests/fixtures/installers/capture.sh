#!/bin/sh
# Capture what installers write to shell startup files.
#   tests/fixtures/installers/capture.sh [tool...]   default: every tool in install_cmd
# Each installer runs in a fresh container as a bash user whose home holds the stock
# /etc/skel files. Changed files land in <tool>/before/ and <tool>/after/ (before/ is
# absent for files the installer created), with the home directory replaced by @HOME@.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
export DOCKER_DEFAULT_PLATFORM=linux/amd64
tools="bun uv rustup nvm conda gcloud homebrew"
rc_files=".profile .bashrc .bash_profile .bash_login .zshenv .zprofile .zshrc .zlogin .config/fish/config.fish .config/fish/conf.d"

# The commands below run inside the container, so ~ and $(...) expand there.
# shellcheck disable=SC2016,SC2088
install_cmd() {
  case "$1" in
    bun) echo 'curl -fsSL https://bun.sh/install | bash' ;;
    uv) echo 'curl -LsSf https://astral.sh/uv/install.sh | sh' ;;
    rustup) echo 'curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal' ;;
    nvm) echo 'curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.41.0/install.sh | bash' ;;
    conda) echo 'curl -fsSLo /tmp/mf.sh https://github.com/conda-forge/miniforge/releases/latest/download/Miniforge3-Linux-x86_64.sh && bash /tmp/mf.sh -b -p ~/miniforge3 && ~/miniforge3/bin/conda init bash zsh fish' ;;
    gcloud) echo 'curl -fsSL https://dl.google.com/dl/cloudsdk/channels/rapid/downloads/google-cloud-cli-linux-x86_64.tar.gz | tar -xz -C ~ && ~/google-cloud-sdk/install.sh --quiet --usage-reporting false --path-update true --command-completion true --rc-path ~/.bashrc' ;;
    # The installer only prints the lines to add; run them the way its "Next steps" says.
    homebrew) echo 'NONINTERACTIVE=1 bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)" | tee /tmp/brew.log && grep -E "^ +echo .*>> " /tmp/brew.log | sh' ;;
    *) return 1 ;;
  esac
}

# shellcheck disable=SC2088
version_cmd() {
  case "$1" in
    bun) echo '~/.bun/bin/bun --version' ;;
    uv) echo '~/.local/bin/uv --version' ;;
    rustup) echo '~/.cargo/bin/rustup --version 2>/dev/null' ;;
    nvm) echo '. ~/.nvm/nvm.sh && nvm --version' ;;
    conda) echo '~/miniforge3/bin/conda --version' ;;
    gcloud) echo '~/google-cloud-sdk/bin/gcloud --version | head -n 1' ;;
    homebrew) echo '/home/linuxbrew/.linuxbrew/bin/brew --version | head -n 1' ;;
  esac
}

# shellcheck disable=SC2086 # tools is a word list
[ $# -gt 0 ] || set -- ${tools}
docker build -q -t rosamta-capture -f "${here}/Dockerfile" "${here}" >/dev/null

for tool in "$@"; do
  cmd=$(install_cmd "${tool}") || { echo "unknown tool: ${tool}" >&2; exit 1; }
  ver=$(version_cmd "${tool}")
  echo "== ${tool}"
  work=$(mktemp -d)
  docker run --rm -v "${work}:/out" rosamta-capture bash -c "
    set -eo pipefail
    snap() { mkdir -p /out/\$1; for f in ${rc_files}; do [ -e \"\$f\" ] && cp -R --parents \"\$f\" /out/\$1/; done; true; }
    snap before
    ( ${cmd} ) || { echo \"install failed: ${tool}\" >&2; exit 1; }
    snap after
    { echo \"captured: \$(date -u +%Y-%m-%d)\"; echo \"version: \$(${ver})\"; echo 'command: ${cmd}'; } > /out/meta
  " >/dev/null

  out="${here}/${tool}"
  rm -rf "${out}"
  mkdir -p "${out}"
  sed 's#/home/me#@HOME@#g' "${work}/meta" > "${out}/meta"
  (cd "${work}/after" && find . -type f) | while read -r f; do
    if [ -f "${work}/before/${f}" ] && cmp -s "${work}/before/${f}" "${work}/after/${f}"; then
      continue
    fi
    for side in before after; do
      [ -f "${work}/${side}/${f}" ] || continue
      mkdir -p "$(dirname "${out}/${side}/${f}")"
      sed 's#/home/me#@HOME@#g' "${work}/${side}/${f}" > "${out}/${side}/${f}"
    done
  done
  rm -rf "${work}"
  (cd "${out}" && find . -type f | sort)
done
