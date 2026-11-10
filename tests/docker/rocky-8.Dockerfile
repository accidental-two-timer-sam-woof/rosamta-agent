FROM rockylinux/rockylinux:8
# zsh on EL8 is 5.5, below the supported 5.8; fish is not packaged
ENV ROSAMTA_SHELLS="sh bash"
