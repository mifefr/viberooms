# VibeRooms server. docker build -t viberooms . && docker run -p 8790:8790 -v viberooms:/data viberooms
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p viberooms

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home /data viberooms && mkdir /data && chown viberooms /data
COPY --from=build /src/target/release/viberooms /usr/local/bin/viberooms
USER viberooms
ENV VR_DATA_DIR=/data VR_HOST=0.0.0.0 VR_PORT=8790
VOLUME /data
EXPOSE 8790
CMD ["viberooms"]
