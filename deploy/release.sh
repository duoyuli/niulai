#!/usr/bin/env bash
# 发布新版本：按今天的日期（上海时间）打标签并推送，CI 随后检查代码、构建镜像。
# 版本号是 vYYYY.MM.DD；同一天再发布时加补丁号 vYYYY.MM.DD.1、vYYYY.MM.DD.2……
# 用法（在仓库任意目录）：deploy/release.sh "这个版本的一句话说明"
set -euo pipefail

msg=${1:?用法：deploy/release.sh "这个版本的一句话说明"}
cd "$(git rev-parse --show-toplevel)"
git fetch -q --tags origin main

# 只发布已经推送到 origin/main 的提交：镜像对应的代码在 GitHub 上看得到
[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || { echo "请切到 main 再发布" >&2; exit 1; }
[ -z "$(git status --porcelain --untracked-files=no)" ] || { echo "有没有提交的改动，先提交" >&2; exit 1; }
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || { echo "本地 main 和 origin/main 不一致，先推送或拉取" >&2; exit 1; }

day=$(TZ=Asia/Shanghai date +%Y.%m.%d)
tag="v$day"
n=0
while git rev-parse -q --verify "refs/tags/$tag" >/dev/null; do
  n=$((n + 1))
  tag="v$day.$n"
done

git tag -a "$tag" -m "niulai ${tag#v}: $msg"
git push -q origin "$tag"
echo "已发布 ${tag}。镜像构建进度见 GitHub Actions，完成后 ghcr.io/duoyuli/niulai:${tag#v} 可以拉取"
