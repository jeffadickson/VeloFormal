# Multi-stage batteries-included VeloFormal container (veloformal + vf-lsp + z3 + yosys-abc)
FROM ubuntu:24.04 AS builder

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake ninja-build git curl ca-certificates pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /src
COPY . .
RUN cargo build --release --bin veloformal --bin vf-lsp

FROM ubuntu:24.04 AS runtime
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    z3 yosys ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/target/release/veloformal /usr/local/bin/veloformal
COPY --from=builder /src/target/release/vf-lsp /usr/local/bin/vf-lsp
COPY --from=builder /src/vip /usr/local/share/veloformal/vip

WORKDIR /work
ENTRYPOINT ["veloformal"]
CMD ["--help"]
