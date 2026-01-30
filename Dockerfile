FROM rust:1-bookworm AS builder

WORKDIR /src

RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    ca-certificates \
    clang \
    libclang-dev \
    pkg-config \
  && rm -rf /var/lib/apt/lists/*

COPY . .

RUN cargo build --release -p fresh-editor


FROM debian:bookworm-slim

RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/fresh /usr/local/bin/fresh

WORKDIR /work

ENTRYPOINT ["fresh"]
