# ADR：旧部署方案（已被取代）

- 状态：已被取代（superseded），2026-10-16。
- 取代方案：[ADR：CI 构建镜像，部署只交付服务本身](ci_cd.md)、[ADR：网页登录](web_login.md)、[ADR：多用户登录与角色](multi_user_login.md) 和 [ADR：Docker 部署的一键升级](one_click_upgrade.md)。
- 当前操作步骤：[运维手册](../operations/runbook.md#deployment)；本文不提供可执行部署命令。

## 历史决定

早期部署使用 Caddy 提供 HTTPS 和基本认证，Compose 通过 profile 管理数据库和代理，并配套安装、升级、备份和恢复脚本。该方案后来被移除，原因是部署组件和认知成本过高，且登录、备份、升级的边界需要由 niulai 与运行环境分别负责。

## 保留的背景

早期方案验证了多架构镜像、内置 PostgreSQL、备份后升级和失败恢复这些需求。当前实现把这些需求拆到发布工作流、应用内登录、`upgrader` 容器和运维手册中；旧方案中的 Caddy、脚本、单一共享账户和历史版本号均不再适用。
