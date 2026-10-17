# ADR：Docker 部署的一键升级

- 状态：已采纳（accepted），2026-10-16。部分取代 [ADR：CI 构建镜像，部署只交付服务本身](ci_cd.md) 决策第 5 条「不提供运维脚本」中与升级有关的部分。升级的顺序（先换掉 upgrader 再迁移、启动新版本）、部署迁移和进度显示已被 [ADR：部署迁移与升级进度](deploy_migrations.md) 取代；版本号格式已被 [ADR：版本号按发布日期定](calendar_versioning.md) 取代。
- 相关文档：[一键升级](../operations/runbook.md#one_click_upgrade)、[ADR：只在发布版本标签时构建镜像](image_on_tag.md)。

## 背景

用户希望像 TokenRouter 那样：页面左上角检测新版本，点「一键升级」就完成升级。TokenRouter 的一键升级是替换自身的预编译程序，要求程序由 systemd 托管；它的 Docker 部署只显示手动命令。niulai 没有预编译程序，用户要求只做 Docker 部署，并且不新增预编译程序这种部署方式。

升级 Docker 部署要拉新镜像并重建 niulai 容器。容器不能重建自己：停掉旧容器时，正在执行升级的进程也会结束。所以需要容器外的一个执行者，它必须能操作 Docker。

## 决策

1. **升级逻辑在 niulai 程序里。** 子命令 `niulai upgrader` 通过 Unix socket 直接调用 Docker Engine API：拉取镜像、在 postgres 容器中执行 `pg_dump`、按原容器的配置用新镜像重建 niulai、等待 healthy、失败时换回。部署目录中只有 `compose.yml` 和 `.env`，没有脚本。
2. **单独的 upgrader 容器运行它。** upgrader 用与 niulai 相同的镜像，以 root 运行，挂载 `/var/run/docker.sock` 和部署目录。由 `.env` 中的 `COMPOSE_PROFILES=upgrader` 启用，默认开启。
3. **提供网页的 niulai 进程不能操作 Docker。** 两者只通过共享卷交换文件：niulai 写要升级到的版本号，upgrader 写心跳、进度和日志。upgrader 只接受 `X.Y.Z` 格式的版本号（2026-10-17 起改为只接受日期版本号，见 [ADR：版本号按发布日期定](calendar_versioning.md)；升级的顺序和部署迁移见 [ADR：部署迁移与升级进度](deploy_migrations.md)），只换发布镜像（`ghcr.io/<所有者>/niulai`）的标签。
4. **先下载、再备份、再切换，失败时换回。** 新镜像下载成功后才动 `.env` 和容器；使用内置数据库时先备份；新容器启动后退出、被重启，或 5 分钟内没有恢复 healthy，就删掉它，换回旧容器和原来的 `.env`。
5. **upgrader 也跟着升级。** niulai 升级成功后，upgrader 用新镜像重建自己，新的 upgrader 启动时删掉旧容器。
6. **版本号编译进镜像。** CI 把 git 标签作为构建参数传入（`NIULAI_VERSION`），niulai 查 GitHub 上最大的正式版本标签来判断有没有新版本。

## 取舍

- **把 Docker 控制接口挂给提供网页的 niulai：** 不采用。网页登录或 niulai 服务一旦被攻破，就等于拿到宿主机的 root 权限。现在网页上最多只能触发「升级到某个已发布的版本」。
- **用 `docker:cli` 镜像执行部署目录中的 shell 脚本：** 第一版这样实现过，发布前改掉了。部署目录多一个要下载、要手动更新的脚本，升级逻辑不随版本发布、没有测试，还多依赖一个镜像。
- **Watchtower 等现成工具：** 不采用。它们按原标签重新拉取镜像，不能把固定的版本号改成新版本，也不会在升级前备份数据库；还多了一个要跟踪维护状况的第三方组件。
- **调用 `docker compose` 重建：** 不采用。要在镜像中加入 Docker CLI 和 Compose 插件（约 100 MB），upgrader 还要读到 `compose.yml`。直接调用 Engine API 时，按旧容器的配置重建，Compose 的标签、卷、端口和网络都保留，之后执行 `docker compose up -d` 也能正常接管。
- **GitHub Releases：** 不使用。项目只打标签，不发 Release，所以用标签列表判断最新版本，改动内容链接到两个标签的对比页面。

## 后果

- upgrader 容器能管理宿主机上的全部容器。不需要一键升级时，删掉 `COMPOSE_PROFILES=upgrader`。
- 本机构建的 `niulai:local` 和开发版不能一键升级。
- 一键升级只换镜像，不改 `compose.yml`。新版本需要改 `compose.yml` 时，要在发布说明中写明，由管理员手动更新。
- 重建的容器带着旧的 Compose 配置哈希，下一次 `docker compose up -d` 会再重建一次，结果相同。
