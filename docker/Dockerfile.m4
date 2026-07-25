FROM rust:1.97-bookworm AS builder

ENV RUSTUP_TOOLCHAIN=1.97.1
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p noema-m4-demo

FROM debian:bookworm-slim

COPY --from=builder /src/target/release/noema-m4-demo /usr/bin/noema-m4-demo
USER 65534:65534
ENTRYPOINT ["/usr/bin/noema-m4-demo"]
