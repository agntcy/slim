FROM rust:1.86 as builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin publisher --bin subscriber --bin workload-gen -p testing

FROM debian:bookworm-slim as testutils
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/workload-gen /app/workload-gen
COPY --from=builder /app/target/release/publisher /app/publisher
COPY --from=builder /app/target/release/subscriber /app/subscriber

CMD ["/app/workload-gen"]
