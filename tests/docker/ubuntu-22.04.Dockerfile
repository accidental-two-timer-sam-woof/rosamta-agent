FROM ubuntu:22.04
ENV ROSAMTA_SHELLS="sh bash zsh fish"
# jammy ships fish 3.3, below the supported 3.6; fish's static build avoids the Launchpad PPA
RUN apt-get update \
 && apt-get install -y --no-install-recommends zsh git curl ca-certificates xz-utils \
 && arch=$(uname -m) \
 && case "${arch}" in \
      x86_64) sum=8f643d10ad1abb0ef7072764c01ceb5cf61b2d373fda400cf60f0bad69b7e095 ;; \
      aarch64) sum=57349f0111728c64b703a84e611aa97ff4d1fa8466385d64e67bd5bc3ffa49fc ;; \
    esac \
 && curl -fsSLo /tmp/fish.tar.xz "https://github.com/fish-shell/fish-shell/releases/download/4.9.3/fish-4.9.3-linux-${arch}.tar.xz" \
 && echo "${sum}  /tmp/fish.tar.xz" | sha256sum -c - \
 && tar -xJf /tmp/fish.tar.xz -C /usr/local/bin fish \
 && rm -rf /tmp/fish.tar.xz /var/lib/apt/lists/*
