# Builds termlab-server from source:
#   docker build -f crates/termlab-server/Dockerfile -t termlab-server .
# Releases publish a ready image instead: ghcr.io/pascallieverse/termlab-server.

FROM node:22-alpine AS web
WORKDIR /src/apps/server-web
COPY apps/server-web/package.json apps/server-web/package-lock.json ./
RUN npm ci
COPY apps/server-web ./
RUN npm run build

FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
COPY --from=web /src/apps/server-web/dist apps/server-web/dist
RUN cargo build --release --locked -p termlab-server \
 && mkdir -p /out/data \
 && cp target/release/termlab-server /out/

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=build /out/termlab-server /termlab-server
COPY --from=build --chown=65532:65532 /out/data /data
ENV TERMLAB_LISTEN=0.0.0.0:8484 \
    TERMLAB_DATA_DIR=/data
VOLUME /data
EXPOSE 8484
USER nonroot
ENTRYPOINT ["/termlab-server"]
CMD ["serve"]
