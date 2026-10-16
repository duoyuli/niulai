#!/bin/sh
# niulai 一键升级助手，在 compose.yml 的 upgrader 容器中运行（docker:cli 镜像，/deploy 是 compose.yml 所在目录）。
# niulai 在共享目录 /upgrade 中写入 request（要升级到的版本号，例如 1.2.0）后，依次：
#   1. 下载新镜像（下载失败时什么都不改）
#   2. 备份内置数据库到 backups/（DATABASE_URL 指向内置数据库时），把 .env 中的 NIULAI_IMAGE 改成新版本
#   3. 重建 niulai 容器，等它恢复 healthy；失败时把 NIULAI_IMAGE 改回原来的版本并重建
# 进度写在 /upgrade/status，输出写在 /upgrade/log。规则见 docs/operations/runbook.md#one_click_upgrade。
set -u
U=/upgrade
NIULAI_UID=65534 # niulai 容器以 nobody 运行，要能在共享目录中写请求
cd /deploy || exit 1
mkdir -p "$U"
chown "$NIULAI_UID:$NIULAI_UID" "$U"

status() { # 状态 目标版本 说明
  date +%s > "$U/heartbeat"
  printf '{"state":"%s","target":"%s","message":"%s","at":%s}\n' "$1" "$2" "$3" "$(date +%s)" > "$U/status.tmp"
  mv "$U/status.tmp" "$U/status"
}

log() { echo "$(date '+%F %T') $*" >> "$U/log"; }

# 改 .env 中的一行：写回原文件而不是替换它，保留文件的属主和权限
set_image() {
  if grep -q '^NIULAI_IMAGE=' .env; then
    sed "s#^NIULAI_IMAGE=.*#NIULAI_IMAGE=$1#" .env > "$U/env.tmp" && cat "$U/env.tmp" > .env && rm -f "$U/env.tmp"
  else
    echo "NIULAI_IMAGE=$1" >> .env
  fi
}

# 等 niulai 恢复 healthy，最多 5 分钟（停机等待 2 分钟加启动和迁移）
wait_healthy() {
  i=0
  while [ $i -lt 100 ]; do
    id=$(docker compose ps -q niulai 2>/dev/null)
    h=$( [ -n "$id" ] && docker inspect -f '{{.State.Health.Status}}' "$id" 2>/dev/null)
    [ "$h" = healthy ] && return 0
    date +%s > "$U/heartbeat"
    i=$((i + 1))
    sleep 3
  done
  return 1
}

upgrade() {
  v=$1
  : > "$U/log"
  current=$(grep '^NIULAI_IMAGE=' .env | tail -n 1 | cut -d= -f2-)
  [ -n "$current" ] || current=ghcr.io/duoyuli/niulai:latest
  repo=${current%:*}
  case "$repo" in
    ghcr.io/*/niulai) ;;
    *) status failed "$v" "NIULAI_IMAGE 是 $current，不是发布的镜像，不能一键升级"; return ;;
  esac
  log "从 $current 升级到 $repo:$v"

  status running "$v" "下载新版本"
  if ! docker pull "$repo:$v" >> "$U/log" 2>&1; then
    status failed "$v" "下载 $repo:$v 失败，没有升级"
    return
  fi

  if grep -q '^DATABASE_URL=.*@postgres:' .env; then
    status running "$v" "备份数据库"
    mkdir -p backups
    f="backups/niulai-before-$v-$(date +%Y%m%d-%H%M%S).dump"
    if docker compose exec -T postgres pg_dump -U niulai -Fc niulai > "$f" 2>> "$U/log"; then
      log "已备份到 $f"
    else
      rm -f "$f"
      status failed "$v" "备份数据库失败，没有升级"
      return
    fi
  fi

  status running "$v" "重启服务"
  set_image "$repo:$v"
  if docker compose up -d --no-deps niulai >> "$U/log" 2>&1 && wait_healthy; then
    log "完成"
    status "done" "$v" "已升级到 $v"
    return
  fi
  log "新版本没有正常启动，改回 $current"
  set_image "$current"
  docker compose up -d --no-deps niulai >> "$U/log" 2>&1
  status failed "$v" "新版本没有正常启动，已改回原来的版本；数据库迁移可能已经执行，必要时用 backups/ 中的备份恢复"
}

while :; do
  date +%s > "$U/heartbeat"
  if [ -f "$U/request" ]; then
    v=$(tr -d ' \r\n' < "$U/request")
    rm -f "$U/request"
    if echo "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
      upgrade "$v"
    else
      status failed "" "版本号不合法"
    fi
  fi
  sleep 3
done
