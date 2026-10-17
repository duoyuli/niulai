# niulai 镜像：后端程序和前端页面在同一个镜像里，容器中只运行 `niulai serve`，在 8080 端口同时提供页面和 API。
# CI（.github/workflows/ci.yml）用它构建并推送 ghcr.io/duoyuli/niulai；部署时直接用那里的镜像，见 compose.yml。
# 需要在本机构建时，在仓库根目录执行：docker build -f deploy/Dockerfile -t niulai:local .

# 1. 编译前端：产物是 frontend/dist 中的静态文件
FROM node:24-slim AS web
WORKDIR /src/frontend
RUN corepack enable
COPY frontend/package.json frontend/pnpm-lock.yaml frontend/pnpm-workspace.yaml ./
RUN pnpm install --frozen-lockfile
COPY frontend/ ./
RUN pnpm build

# 2. 编译后端：产物是一个可执行文件 niulai，数据库迁移已编译在里面
FROM rust:1.99-slim AS build
WORKDIR /src
COPY backend/rust-toolchain.toml backend/Cargo.toml backend/Cargo.lock ./
# 先只编译依赖，源码变化时可以复用这一层缓存
RUN mkdir -p src/bin && echo 'fn main() {}' > src/bin/niulai.rs && echo '' > src/lib.rs \
    && cargo build --release --locked && rm -rf src
COPY backend/build.rs ./
COPY backend/src/ src/
COPY backend/migrations/ migrations/
# 版本号编译进程序（CI 传入 git 标签，例如 v2026.10.18；本机构建是 dev），页面据此判断有没有新版本
ARG NIULAI_VERSION=dev
ENV NIULAI_VERSION=${NIULAI_VERSION}
RUN touch src/lib.rs && cargo build --release --locked

# 3. 运行镜像：只包含程序、前端静态文件和数据集配置，不含编译工具和源码
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/target/release/niulai /usr/local/bin/
COPY backend/config/ /app/config/
COPY --from=web /src/frontend/dist/ /app/web/
# 时区固定为上海，与宿主机无关。程序的日期计算、数据库会话、日志时间本身就按上海时间，这里让容器里的其他时间（例如升级备份的文件名）也一致
ENV TZ=Asia/Shanghai \
    NIULAI_DATASETS_PATH=/app/config/datasets.yaml \
    NIULAI_WEB_DIR=/app/web \
    NIULAI_API_BIND=0.0.0.0:8080 \
    NIULAI_LOG_FORMAT=json
USER nobody
EXPOSE 8080
CMD ["niulai", "serve"]
