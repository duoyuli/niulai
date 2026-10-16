#!/usr/bin/env bash
# 从备份恢复数据库，用于升级失败后回滚，或把数据迁到另一台服务器。
# 先恢复到临时库，成功后再与当前库互换，服务正常启动后才删除旧库；任何一步失败，当前库都保持原样。
# 数据库用户需要有建库权限（内置数据库默认满足）。
#
#   ./restore.sh backups/niulai-20261016-033000.dump
#   ./restore.sh -y <备份文件>      # 不询问
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ASSUME_YES=0 FILE=""
for arg in "$@"; do
  case "$arg" in
    -y|--yes) ASSUME_YES=1 ;;
    -*) die "未知参数：$arg" ;;
    *) FILE="$arg" ;;
  esac
done
if [ -z "$FILE" ] || [ ! -f "$FILE" ]; then die "用法：./restore.sh [-y] <备份文件>"; fi

require_docker
load_env
# 连接串拆成「服务器部分」和库名，例如 postgres://u:p@postgres:5432 和 niulai
url="${DATABASE_URL%%\?*}"
server="${url%/*}" db="${url##*/}"
tmp="${db}_restore" old="${db}_before_restore_$(date +%Y%m%d%H%M%S)"
if [ "$ASSUME_YES" -eq 0 ]; then
  read -r -p "将用 ${FILE} 替换数据库 ${db}（原库改名为 ${old} 暂时保留），继续？[y/N] " ok
  [ "$ok" = "y" ] || [ "$ok" = "Y" ] || die "已取消"
fi

# 借用 postgres 镜像里的客户端，在 compose 网络中执行
pg() { dc run --rm --no-deps -T --entrypoint "$1" postgres "${@:2}"; }
admin_sql() { pg psql --quiet --no-psqlrc -v ON_ERROR_STOP=1 --dbname="$server/postgres" -c "$1" > /dev/null; }

has_profile db && dc up -d --wait postgres
info "恢复到临时库 $tmp"
admin_sql "DROP DATABASE IF EXISTS \"$tmp\""
admin_sql "CREATE DATABASE \"$tmp\""
if ! pg pg_restore --no-owner --single-transaction --dbname="$server/$tmp" < "$FILE"; then
  admin_sql "DROP DATABASE IF EXISTS \"$tmp\"" || true
  die "恢复失败，当前库没有改动"
fi

info "停止 niulai，切换数据库"
dc stop niulai
# 失败时把服务启动回来
trap 'warn "切换中断，重新启动 niulai"; dc up -d niulai || true' ERR
admin_sql "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname IN ('$db', '$tmp') AND pid <> pg_backend_pid()"
admin_sql "ALTER DATABASE \"$db\" RENAME TO \"$old\""
admin_sql "ALTER DATABASE \"$tmp\" RENAME TO \"$db\""
dc up -d niulai
wait_healthy 240
trap - ERR

info "删除旧库 $old"
admin_sql "DROP DATABASE \"$old\""
info "恢复完成"
