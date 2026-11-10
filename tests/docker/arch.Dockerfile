FROM archlinux:latest
ENV ROSAMTA_SHELLS="sh bash zsh fish"
# pacman's download sandbox fails under x86_64 emulation (Rosetta on Apple Silicon)
RUN pacman -Syu --noconfirm --disable-sandbox zsh fish && pacman -Scc --noconfirm
