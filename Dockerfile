FROM rust:1.85-bookworm AS build
WORKDIR /src

# Install dependencies for SQLite
RUN apt-get update && apt-get install -y libsqlite3-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock* ./
COPY crates/ crates/

# Build release binary
RUN cargo build --release -p mfs-server

FROM debian:bookworm-slim
WORKDIR /app

# Install runtime SQLite dependency
RUN apt-get update && apt-get install -y libsqlite3-0 ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/mfs-server /app/memfuse-server

ENV MEMFUSE_WORKSPACE_ROOT=/data/workspace
ENV MEMFUSE_SOURCE_KIND=managed
ENV MEMFUSE_BIND_ADDR=0.0.0.0:18720

EXPOSE 18720
ENTRYPOINT ["/app/memfuse-server"]
