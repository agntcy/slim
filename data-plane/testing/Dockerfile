FROM rust:1.86 as builder

RUN DEBIAN_FRONTEND=noninteractive \
    apt-get update && \
    apt-get install --no-install-recommends -y \
        curl \
        file \
        make \
        unzip \
        git \
        pkg-config \
        clang-14 \
        llvm-14

# Install taskfile
RUN sh -c "$(curl --location https://taskfile.dev/install.sh)" -- -d -b /usr/local/bin

WORKDIR /app
COPY . .
RUN task                                                                        \
    data-plane:build                                                            \
    BUILD_ARGS="--bin publisher --bin subscriber --bin workload-gen -p testing" \
    PROFILE=release

FROM debian:bookworm-slim as testutils
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/workload-gen /app/workload-gen
COPY --from=builder /app/target/release/publisher /app/publisher
COPY --from=builder /app/target/release/subscriber /app/subscriber

CMD ["/app/workload-gen"]
