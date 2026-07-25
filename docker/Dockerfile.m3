FROM rust:1.97-bookworm AS builder

ENV RUSTUP_TOOLCHAIN=1.97.1
WORKDIR /src
COPY . .
RUN cargo build --locked --release -p noema-m3-demo -p noema-test-workload

FROM debian:bookworm-slim

RUN mkdir -p /var/lib/noema && chown 65534:65534 /var/lib/noema
COPY --from=builder /src/target/release/noema-m3-demo /usr/bin/noema-m3-demo
COPY --from=builder /src/target/release/noema-test-workload /usr/lib/noema/noema-test-workload

ENV NOEMA_TEST_WORKLOAD=/usr/lib/noema/noema-test-workload
ENV NOEMA_STATE_PATH=/var/lib/noema/state.json
USER 65534:65534
ENTRYPOINT ["/usr/bin/noema-m3-demo"]
