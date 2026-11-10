FROM almalinux:9
ENV ROSAMTA_SHELLS="sh bash zsh fish"
RUN dnf install -y epel-release && dnf install -y zsh fish && dnf clean all
