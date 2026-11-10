FROM ubuntu:24.04
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      bash zsh fish ca-certificates curl git sudo unzip xz-utils procps file \
 && rm -rf /var/lib/apt/lists/* \
 && useradd -m -s /bin/bash me \
 && echo 'me ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/me
USER me
WORKDIR /home/me
ENV SHELL=/bin/bash
