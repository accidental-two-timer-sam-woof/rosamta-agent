FROM fedora:latest
ENV ROSAMTA_SHELLS="sh bash zsh fish"
RUN dnf install -y zsh fish git && dnf clean all
