# Build and runtime image for MacPlayer. The application uses libmpv at runtime;
# the final image therefore includes the development package in the builder and
# the shared library in the runtime image.
FROM rust:1.95-bookworm AS builder

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
         libdbus-1-dev \
         libegl1-mesa-dev \
         libfontconfig1-dev \
         libgl1-mesa-dev \
         libgtk-3-dev \
         libasound2-dev \
         libmpv-dev \
         libpulse-dev \
         libudev-dev \
         libwayland-dev \
         libx11-dev \
         libx11-xcb-dev \
         libxcb1-dev \
         libxcursor-dev \
         libxfixes-dev \
         libxinerama-dev \
         libxi-dev \
         libxkbcommon-dev \
         libxkbcommon-x11-dev \
         libxrandr-dev \
         pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY src ./src
COPY assets ./assets

RUN cargo build --release --locked

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
         libdbus-1-3 \
         libegl1 \
         libfontconfig1 \
         libgl1 \
         libgtk-3-0 \
         libasound2 \
         libmpv2 \
         libpulse0 \
         libudev1 \
         libwayland-client0 \
         libx11-6 \
         libx11-xcb1 \
         libxcb1 \
         libxcursor1 \
         libxfixes3 \
         libxinerama1 \
         libxi6 \
         libxkbcommon0 \
         libxkbcommon-x11-0 \
         libxrandr2 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/macplayer /usr/local/bin/macplayer

ENTRYPOINT ["/usr/local/bin/macplayer"]
