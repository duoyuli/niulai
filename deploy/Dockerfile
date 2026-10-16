# 构建上下文为仓库根目录：docker build -f deploy/Dockerfile -t niulai .
# 一个镜像包含后端程序和前端页面，`niulai serve` 同时提供 API 和页面。

FROM node:24-slim AS web
WORKDIR /src/frontend
RUN corepack enable
COPY frontend/package.json frontend/pnpm-lock.yaml frontend/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY frontend/ ./
RUN pnpm build

FROM rust:1.99-slim AS build
WORKDIR /src
COPY backend/rust-toolchain.toml backend/Cargo.toml backend/Cargo.lock ./
# 先只编译依赖，源码变化时可以复用这一层缓存
RUN mkdir -p src/bin && echo 'fn main() {}' > src/bin/niulai.rs && echo '' > src/lib.rs \
    && cargo build --release --locked && rm -rf src
COPY backend/src/ src/
COPY backend/migrations/ migrations/
RUN touch src/lib.rs && cargo build --release --locked

FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/target/release/niulai /usr/local/bin/
COPY backend/config/ /app/config/
COPY --from=web /src/frontend/dist/ /app/web/
ENV NIULAI_DATASETS_PATH=/app/config/datasets.yaml \
    NIULAI_WEB_DIR=/app/web \
    NIULAI_API_BIND=0.0.0.0:8080 \
    NIULAI_LOG_FORMAT=json
USER nobody
EXPOSE 8080
CMD ["niulai", "serve"]
