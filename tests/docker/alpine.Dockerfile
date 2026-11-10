FROM alpine:latest
ENV ROSAMTA_SHELLS="sh bash zsh fish"
RUN apk add --no-cache bash zsh fish git
