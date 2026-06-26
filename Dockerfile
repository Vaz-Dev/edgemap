FROM rust:trixie AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY . .
RUN cargo fetch
RUN cargo build --release --bin edgemap
RUN ls -lh /app/target/release/edgemap

FROM debian:trixie-slim
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*
RUN useradd -m -u 1000 appuser

WORKDIR /root/
COPY --from=builder /app/target/release/edgemap /root/edgemap
RUN chown appuser:appuser /root/edgemap

EXPOSE 8080
USER appuser
CMD ["./edgemap", "http://localhost:3000"]
