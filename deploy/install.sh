#!/usr/bin/env bash
# 一键安装 niulai：生成 .env 和登录凭据，拉取镜像，启动，等待就绪。
# 同花顺 API Key 不在这里配置：第一次打开网页时按引导填写，保存后自动开始首次全量回补。
# 可以重复执行：已有 .env 时不再询问，只启动并检查。
#
# 交互式：  ./install.sh
# 非交互式：NIULAI_IMAGE=ghcr.io/me/niulai:0.1.0 NIULAI_DOMAIN=niulai.example.com ./install.sh -y
#
# 选项：
#   -y              不询问，缺少的值用环境变量或默认值
#   --no-pull       不拉取镜像（使用本机已有的镜像）
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ASSUME_YES=0 PULL=1
for arg in "$@"; do
  case "$arg" in
    -y|--yes) ASSUME_YES=1 ;;
    --no-pull) PULL=0 ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) die "未知参数：$arg" ;;
  esac
done

# 询问一个值：已有环境变量时直接用；-y 时用默认值
ask() {
  local var="$1" prompt="$2" default="${3:-}" value="${!1:-}"
  if [ -z "$value" ] && [ "$ASSUME_YES" -eq 0 ]; then
    read -r -p "$prompt${default:+（默认 ${default}）}：" value
  fi
  printf -v "$var" '%s' "${value:-$default}"
}

write_env() {
  info "生成配置 .env"
  ask NIULAI_IMAGE "镜像地址，例如 ghcr.io/<github用户名>/niulai:0.1.0"
  [ -n "$NIULAI_IMAGE" ] || die "需要镜像地址（NIULAI_IMAGE）"
  ask NIULAI_DOMAIN "公网域名（要解析到本机；留空表示不对公网开放，用 SSH 隧道访问）"

  local profiles="db" pg_password
  [ -n "$NIULAI_DOMAIN" ] && profiles="db,proxy"
  pg_password="$(random_token 32)"
  umask 077
  cat > .env <<EOF
# 由 install.sh 生成，说明见 .env.example。含密码和 API Key，不要提交或外传。
NIULAI_IMAGE=$NIULAI_IMAGE
COMPOSE_PROFILES=$profiles
POSTGRES_PASSWORD=$pg_password
DATABASE_URL=postgres://niulai:$pg_password@postgres:5432/niulai
NIULAI_DOMAIN=$NIULAI_DOMAIN
EOF
  chmod 600 .env
}

# 网页登录：Caddy 的基本认证，密码只保存 bcrypt 哈希
write_auth() {
  local user password hash
  user="${NIULAI_WEB_USER:-}"
  ask user "网页登录用户名" "admin"
  password="${NIULAI_WEB_PASSWORD:-}"
  [ -n "$password" ] || password="$(random_token 16)"
  hash="$(docker run --rm caddy:2.11.6 caddy hash-password --plaintext "$password")"
  mkdir -p caddy
  umask 077
  printf 'basic_auth {\n\t%s %s\n}\n' "$user" "$hash" > caddy/auth.caddy
  WEB_LOGIN="$user / $password"
}

require_docker
[ -f .env ] || write_env
load_env
WEB_LOGIN=""
if has_profile proxy && [ ! -f caddy/auth.caddy ]; then
  write_auth
fi

if [ "$PULL" -eq 1 ]; then
  info "拉取镜像"
  if ! dc pull --quiet; then
    die "拉取镜像失败。镜像是私有的话，先登录：docker login ghcr.io -u <github用户名>（密码用有 read:packages 权限的 token），或在 GitHub 上把包设为公开"
  fi
fi

info "启动服务（${COMPOSE_PROFILES}）"
dc up -d
# 凭据文件是挂载进去的，内容变了 Caddy 不会自动重新加载
[ -n "$WEB_LOGIN" ] && dc restart caddy >/dev/null
wait_healthy 240

echo
info "安装完成"
if has_profile proxy; then
  echo "  访问地址：https://$NIULAI_DOMAIN/"
  [ -n "$WEB_LOGIN" ] && echo "  登录：${WEB_LOGIN}（只显示这一次；忘记时删除 caddy/auth.caddy 后重新运行 ./install.sh）"
  echo "  证书由 Caddy 自动申请：域名要解析到本机，80、443 端口要放行。"
else
  echo "  只监听本机：ssh -L ${NIULAI_PORT:-8080}:127.0.0.1:${NIULAI_PORT:-8080} <服务器>，然后打开 http://127.0.0.1:${NIULAI_PORT:-8080}/"
fi
echo "  第一次打开网页时，按引导填写同花顺金融数据服务的 API Key（在 https://fuyao.aicubes.cn/admin/ 生成），"
echo "  保存后自动开始首次全量回补（约 51 小时）。进度见页面「数据同步」。"
echo "  每日备份：把 $DEPLOY_DIR/backup.sh 加入 crontab，见 docs/operations/runbook.md#deployment"
