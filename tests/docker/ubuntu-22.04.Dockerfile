FROM ubuntu:22.04
ENV ROSAMTA_SHELLS="sh bash zsh fish"
# jammy ships fish 3.3, below the supported 3.6
RUN apt-get update \
 && apt-get install -y --no-install-recommends software-properties-common gpg-agent \
 && add-apt-repository -y ppa:fish-shell/release-4 \
 && apt-get install -y --no-install-recommends zsh fish \
 && rm -rf /var/lib/apt/lists/*
