# ADR：Docker 部署的一键升级

- 状态：已采纳（accepted），2026-10-16。部分取代 [ADR：CI 构建镜像，部署只交付服务本身](ci_cd.md) 决策第 5 条「不提供运维脚本」。
- 相关文档：[一键升级](../operations/runbook.md#one_click_upgrade)、[ADR：只在发布版本标签时构建镜像](image_on_tag.md)。

## 背景

用户希望像 TokenRouter 那样：页面左上角检测新版本，点「一键升级」就完成升级。TokenRouter 的一键升级是替换自身的预编译程序，要求程序由 systemd 托管；它的 Docker 部署只显示手动命令。niulai 没有预编译程序，用户要求只做 Docker 部署，并且不新增预编译程序这种部署方式。

升级 Docker 部署要拉新镜像并重建 niulai 容器。容器不能重建自己：停掉旧容器时，正在执行升级的进程也会结束。所以需要容器外的一个执行者，它必须能操作 Docker。

## 决策

1. **单独的 upgrader 容器执行升级。** 它用官方 `docker:cli` 镜像，挂载 `/var/run/docker.sock` 和 `compose.yml` 所在目录，运行仓库中的 `deploy/upgrader.sh`。由 `.env` 中的 `COMPOSE_PROFILES=upgrader` 启用，默认开启。
2. **niulai 不能操作 Docker。** 两者只通过共享卷交换文件：niulai 写要升级到的版本号，upgrader 写心跳、进度和日志。upgrader 只接受 `X.Y.Z` 格式的版本号，只换 niulai 镜像的标签。
3. **脚本在部署目录中，不在 niulai 镜像中。** 挂着 Docker 控制接口执行的代码，只来自管理员放到服务器上的文件，不来自 niulai 能写的位置。
4. **先下载、再备份、再切换，失败时换回。** 新镜像下载成功后才动 `.env`；使用内置数据库时先 `pg_dump`；新版本 5 分钟内没有恢复 `healthy`，就把镜像改回原来的版本。
5. **版本号编译进镜像。** CI 把 git 标签作为构建参数传入（`NIULAI_VERSION`），niulai 查 GitHub 上最大的正式版本标签来判断有没有新版本。

## 取舍

- **把 Docker 控制接口挂给 niulai：** 不采用。网页登录或 niulai 本身一旦被攻破，就等于拿到宿主机的 root 权限。现在网页上最多只能触发「升级到某个已发布的版本」。
- **Watchtower 等现成工具：** 不采用。它们按原标签重新拉取镜像，不能把固定的版本号改成新版本，也不会在升级前备份数据库；还多了一个要跟踪维护状况的第三方组件。
- **upgrader 的脚本放进 niulai 镜像，随版本自动更新：** 不采用，原因见决策第 3 条。代价是 `upgrader.sh` 和 `compose.yml` 自身的变化要手动更新。
- **GitHub Releases：** 不使用。项目只打标签，不发 Release，所以用标签列表判断最新版本，改动内容链接到两个标签的对比页面。

## 后果

- upgrader 容器能管理宿主机上的全部容器。不需要一键升级时，删掉 `COMPOSE_PROFILES=upgrader`。
- 本机构建的 `niulai:local` 和开发版不能一键升级。
- 新版本修改了 `compose.yml` 或 `upgrader.sh` 时，要在发布说明中写明，由管理员手动更新这两个文件。
