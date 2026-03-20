# syntax=docker/dockerfile:1.7

FROM rust:1.93-bookworm AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      clang \
      cmake \
      g++ \
      make \
      perl \
      pkg-config && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock build.rs ./
COPY schemas ./schemas
COPY src ./src

RUN cargo build --locked --release --bin dbt-nova

FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      libstdc++6 && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --uid 10001 nova

ENV DBT_NOVA_SERVER_TRANSPORT=streamable_http \
    DBT_NOVA_HTTP_PATH=/mcp \
    DBT_NOVA_STORAGE_DIR=/tmp/dbt-nova \
    DBT_NOVA_EMBEDDINGS_CACHE_DIR=/tmp/dbt-nova/models \
    PORT=8080

COPY --from=builder /app/target/release/dbt-nova /usr/local/bin/dbt-nova

USER 10001:10001
WORKDIR /home/nova

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/dbt-nova", "server", "start"]
