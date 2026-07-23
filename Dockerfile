FROM rust:1-slim-bookworm AS build
WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

# From-scratch code search engine (own trigram index, own line matcher — no vendored search
# library). Purely in-memory: no disk, no git, no auth/credential handling of any kind —
# content and credentials are the caller's problem; this only ever speaks HTTP.
FROM debian:bookworm-slim
WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends wget \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/xgrep-server /usr/local/bin/xgrep-server

ENV PORT=7777
EXPOSE 7777

HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 \
  CMD wget -q -O- http://localhost:7777/health || exit 1

ENTRYPOINT ["xgrep-server"]
