# Build matchbox_server from crates.io. Pinned to 0.10 to match the
# bevy_matchbox version used by the client (Cargo.toml). When you upgrade
# bevy_matchbox in the client, bump this version to the matching release.
FROM rust:1.85-slim AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
RUN cargo install matchbox_server --version ^0.10 --root /out

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /out/bin/matchbox_server /usr/local/bin/matchbox_server
EXPOSE 3536
# matchbox_server takes a single positional host:port arg. 0.0.0.0 binds
# all interfaces so Fly's edge can route external traffic in.
CMD ["matchbox_server", "0.0.0.0:3536"]
