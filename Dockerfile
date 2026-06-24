# syntax=docker/dockerfile:1.7

FROM rust:1.93-bookworm@sha256:7c4ae649a84014c467d79319bbf17ce2632ae8b8be123ac2fb2ea5be46823f31 AS builder

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
COPY benches ./benches
COPY schemas ./schemas
COPY src ./src
COPY vendor ./vendor

RUN cargo build --locked --release --bin dbt-nova

FROM debian:bookworm-slim@sha256:67b30a61dc87758f0caf819646104f29ecbda97d920aaf5edc834128ac8493d3 AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      libstdc++6 && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --system --create-home --uid 10001 nova

ENV DBT_NOVA_SERVER_TRANSPORT=streamable_http \
    DBT_NOVA_HTTP_PATH=/mcp \
    DBT_NOVA_TOOL_DENYLIST=execute_sql,run_recipe \
    DBT_NOVA_STORAGE_DIR=/tmp/dbt-nova \
    DBT_NOVA_EMBEDDINGS_CACHE_DIR=/tmp/dbt-nova/models \
    PORT=8080

COPY --from=builder /app/target/release/dbt-nova /usr/local/bin/dbt-nova

USER 10001:10001
WORKDIR /home/nova

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -fsS "http://127.0.0.1:${PORT:-8080}/healthz" >/dev/null || exit 1

ENTRYPOINT ["/usr/local/bin/dbt-nova", "server", "start"]
