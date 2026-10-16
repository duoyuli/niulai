# install.sh、upgrade.sh、backup.sh、restore.sh 共用的函数。不单独执行。
# shellcheck shell=bash

DEPLOY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DEPLOY_DIR" || exit 1

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m警告：\033[0m%s\n' "$*" >&2; }
die() { printf '\033[1;31m错误：\033[0m%s\n' "$*" >&2; exit 1; }

dc() { docker compose "$@"; }

require_docker() {
  command -v docker >/dev/null || die "没有找到 docker，先安装 Docker Engine：https://docs.docker.com/engine/install/"
  docker info >/dev/null 2>&1 || die "无法连接 Docker：确认 Docker 正在运行，当前用户有权限（或用 sudo 执行）"
  local v
  v="$(docker compose version --short 2>/dev/null)" || die "没有找到 Docker Compose 插件（docker compose），先安装：https://docs.docker.com/compose/install/"
  # depends_on 的 required: false 需要 2.20 以上
  local major="${v%%.*}" rest="${v#*.}"
  major="${major#v}"
  local minor="${rest%%.*}"
  if [ "$major" -lt 2 ] || { [ "$major" -eq 2 ] && [ "$minor" -lt 20 ]; }; then
    die "Docker Compose 版本 $v 太旧，需要 2.20 或以上"
  fi
}

load_env() {
  [ -f .env ] || die "没有 $DEPLOY_DIR/.env，先运行 ./install.sh"
  set -a
  # shellcheck disable=SC1091
  . ./.env
  set +a
}

# 当前 .env 是否启用了某个 profile
has_profile() { case ",${COMPOSE_PROFILES:-}," in *",$1,"*) return 0 ;; *) return 1 ;; esac; }

# 等 niulai 的健康检查通过，最多等 $1 秒
wait_healthy() {
  local timeout="${1:-180}" waited=0 cid status
  info "等待服务就绪（最多 ${timeout} 秒）"
  while [ "$waited" -lt "$timeout" ]; do
    cid="$(dc ps -q niulai 2>/dev/null || true)"
    if [ -n "$cid" ]; then
      status="$(docker inspect -f '{{.State.Health.Status}}' "$cid" 2>/dev/null || true)"
      [ "$status" = "healthy" ] && { info "服务已就绪"; return 0; }
    fi
    sleep 5
    waited=$((waited + 5))
  done
  dc logs --tail 50 niulai >&2 || true
  die "服务在 ${timeout} 秒内没有就绪，上面是最近的日志"
}

# 不含引号、空白和 $ 的随机字符串，长度 $1
random_token() {
  LC_ALL=C tr -dc 'A-Za-z0-9' < <(head -c 512 /dev/urandom) | cut -c "1-${1:-24}"
}
