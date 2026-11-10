FROM debian:13
ENV ROSAMTA_SHELLS="sh bash zsh fish"
RUN apt-get update \
 && apt-get install -y --no-install-recommends zsh fish \
 && rm -rf /var/lib/apt/lists/*
