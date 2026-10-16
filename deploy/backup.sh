#!/usr/bin/env bash
# 备份数据库到 backups/niulai-<时间>.dump（pg_dump 自定义格式，已压缩），保留最近 NIULAI_BACKUP_KEEP 份（默认 7）。
# 内置数据库和外部数据库（.env 中的 DATABASE_URL）都适用。每天执行一次，例如 crontab：
#   30 3 * * * /opt/niulai/deploy/backup.sh >> /opt/niulai/deploy/backups/backup.log 2>&1
set -euo pipefail
. "$(dirname "$0")/lib.sh"

require_docker
load_env
mkdir -p backups
file="backups/niulai-$(date +%Y%m%d-%H%M%S).dump"
info "备份到 $file"
# 借用 postgres 镜像里的 pg_dump；在 compose 网络中执行，内置数据库的主机名 postgres 可以解析
dc run --rm --no-deps -T --entrypoint pg_dump postgres --format=custom --dbname="$DATABASE_URL" > "$file.partial"
mv "$file.partial" "$file"
info "完成：$(du -h "$file" | cut -f1)"

keep="${NIULAI_BACKUP_KEEP:-7}"
# 文件名由本脚本生成，只含数字和连字符，按修改时间排序用 ls 即可
# shellcheck disable=SC2012
ls -1t backups/niulai-*.dump | tail -n +"$((keep + 1))" | while read -r old; do
  info "删除旧备份 $old"
  rm -f -- "$old"
done
