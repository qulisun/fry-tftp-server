FROM rust:1.94-slim AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --no-default-features

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/fry-tftp-server /usr/local/bin/fry-tftp-server
COPY --from=builder /build/config/default.toml /etc/fry-tftp-server/config.toml

EXPOSE 69/udp
VOLUME /srv/tftp

# TFTP uses ephemeral ports per session. Recommended: docker run --net=host
# Alternative: -p 69:69/udp -p 10000-60000:10000-60000/udp
ENTRYPOINT ["fry-tftp-server", "--headless", "-c", "/etc/fry-tftp-server/config.toml"]
