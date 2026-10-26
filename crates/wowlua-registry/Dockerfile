# syntax=docker/dockerfile:1
#
# The wowlua registry server. Build from the repository root (the server
# shares crates of the workspace):
#
#   docker build -f crates/wowlua-registry/Dockerfile -t wowlua-registry .
#
# It serves plain HTTP on 8080 (HTTPS ends at the proxy in front of it) and
# keeps everything in /data, a volume. The environment variables are in
# crates/wowlua-registry/README.md.

FROM rust:1.96-slim-trixie AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    CARGO_INCREMENTAL=0 cargo build --release --locked -p wowlua-registry \
    && cp target/release/wowlua-registry /wowlua-registry \
    && mkdir /data

# glibc, libgcc and CA certificates, no shell; runs as uid 65532.
FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /wowlua-registry /wowlua-registry
# A new named volume takes this folder's owner, so the server can write it.
COPY --from=build --chown=65532:65532 /data /data
ENV WOWLUA_REGISTRY_ADDR=0.0.0.0:8080 \
    WOWLUA_REGISTRY_DATA=/data
EXPOSE 8080
VOLUME ["/data"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD ["/wowlua-registry", "health"]
ENTRYPOINT ["/wowlua-registry"]
