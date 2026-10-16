#!/usr/bin/env bash
# 升级：先备份数据库，再拉取 .env 中 NIULAI_IMAGE 指定的镜像并重启。新版本启动时自动执行数据库迁移。
# 升级到指定版本：先把 .env 中的 NIULAI_IMAGE 改成新的版本号，再运行本脚本。
#
# 回滚：迁移只能向前执行，旧版本程序不一定能用新的表结构。把 NIULAI_IMAGE 改回旧版本，
# 再用升级前的备份恢复：./restore.sh backups/<升级前的备份>.dump
#
#   ./upgrade.sh
#   ./upgrade.sh --no-backup
#   ./upgrade.sh --no-pull      # 镜像已在本机（docker load 或本机构建）
set -euo pipefail
. "$(dirname "$0")/lib.sh"

BACKUP=1 PULL=1
for arg in "$@"; do
  case "$arg" in
    --no-backup) BACKUP=0 ;;
    --no-pull) PULL=0 ;;
    *) die "未知参数：$arg" ;;
  esac
done

require_docker
load_env
old="$(docker inspect -f '{{.Image}}' "$(dc ps -a -q niulai)" 2>/dev/null || true)"
[ "$BACKUP" -eq 1 ] && "$DEPLOY_DIR/backup.sh"
if [ "$PULL" -eq 1 ]; then
  info "拉取 $NIULAI_IMAGE"
  dc pull --quiet
fi
info "重启服务（等正在执行的同步任务结束，最多 2 分钟）"
dc up -d
wait_healthy 240
new="$(docker inspect -f '{{.Image}}' "$(dc ps -q niulai)")"
if [ "$old" = "$new" ]; then
  info "镜像没有变化，已是最新"
elif [ -z "$old" ]; then
  info "已启动：${new:0:19}"
else
  info "已升级：${old:0:19} → ${new:0:19}"
fi
