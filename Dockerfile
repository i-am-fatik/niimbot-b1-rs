FROM --platform=$BUILDPLATFORM rust:1.99.1-trixie AS build
RUN rustup target add wasm32-unknown-unknown \
	&& cargo install wasm-pack --version 0.14.0 --locked
WORKDIR /src
COPY . .
RUN wasm-pack build niimbot-b1-pwa --release --target web --out-dir www/pkg --no-typescript --no-pack

FROM nginxinc/nginx-unprivileged:1.30.6-alpine-slim
LABEL org.opencontainers.image.source=https://github.com/i-am-fatik/niimbot-b1-rs
COPY niimbot-b1-pwa/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /src/niimbot-b1-pwa/www /usr/share/nginx/html
