FROM ubuntu:22.04
ENV ROSAMTA_SHELLS="sh bash zsh fish"
# jammy ships fish 3.3, below the supported 3.6; fish's static build avoids the Launchpad PPA
ARG FISH=4.9.3
RUN apt-get update \
 && apt-get install -y --no-install-recommends zsh git curl ca-certificates xz-utils \
 && curl -fsSL "https://github.com/fish-shell/fish-shell/releases/download/${FISH}/fish-${FISH}-linux-$(uname -m).tar.xz" \
  | tar -xJ -C /usr/local/bin fish \
 && rm -rf /var/lib/apt/lists/*
