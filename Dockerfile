# A statically-linked musl binary, built in CI and copied in.
#
# Deliberately not compiled inside the image: the multi-arch build would then need QEMU to compile
# aarch64 on an x86 runner, which turns minutes into most of an hour. CI already builds every
# release target, so buildx just assembles what it made.
#
#   docker buildx build --platform linux/amd64,linux/arm64 \
#     --build-arg BIN_DIR=dist -t ghcr.io/brooklyn/terrustia .

FROM alpine:3.21

# Alpine rather than scratch. A game server people run is a game server people eventually need to
# get inside — to look at a world file, check a permission, or work out why a save failed — and
# eleven megabytes of shell is a fair price for that.
RUN apk add --no-cache ca-certificates tini && \
    adduser -D -u 10001 -h /data terrustia

ARG TARGETARCH
ARG BIN_DIR=dist
COPY ${BIN_DIR}/terrustia-linux-${TARGETARCH} /usr/local/bin/terrustia
RUN chmod +x /usr/local/bin/terrustia

# The world lives on a volume, not in the image. Without this every `docker run` would generate a
# world, serve it, and throw it away.
VOLUME ["/data"]
WORKDIR /data
USER terrustia

EXPOSE 7777/tcp

# Deliberately no terrustia.toml. It is the default config filename, so baking one in would have
# every container silently pick up settings nobody chose.
ENV TERRUSTIA_LOG=info

# A TCP connect is the only honest check available: the protocol has no ping that does not require
# a handshake, and a server that accepts sockets is a server whose accept loop and game task are
# both alive. `start-period` is generous because generating a large world takes a while.
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=3 \
    CMD nc -z 127.0.0.1 7777 || exit 1

# tini reaps zombies and, more usefully here, forwards SIGTERM — without which `docker stop` would
# kill the server outright and skip the shutdown save.
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/terrustia"]
CMD ["--listen", "0.0.0.0:7777"]
