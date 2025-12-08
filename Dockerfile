FROM --platform=$BUILDPLATFORM ghcr.io/rust-cross/cargo-zigbuild:0.20.1 AS builder

WORKDIR /build

RUN apt update && apt install -y clang python3-dev

COPY ./Cargo.toml ./Cargo.lock ./
RUN mkdir src && echo "" > src/lib.rs
RUN cargo fetch
RUN rm -rf src

COPY . .
RUN cargo zigbuild --target aarch64-unknown-linux-gnu

RUN cp /build/target/aarch64-unknown-linux-gnu/debug/libpyvoy.so /build/arm64_libpyvoy.so

##### Build the final image #####
FROM envoyproxy/envoy:v1.36.3 AS envoy
ARG TARGETARCH
ENV ENVOY_DYNAMIC_MODULES_SEARCH_PATH=/usr/local/lib
COPY --from=builder /build/${TARGETARCH}_libpyvoy.so /usr/local/lib/libpyvoy.so
