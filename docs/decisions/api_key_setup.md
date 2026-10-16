# ADR：API Key 在首次登录引导中配置

- 状态：已采纳（accepted），2026-10-16。取代 [ADR：部署方案](deployment.md) 决策第 4 条中「`install.sh` 询问 API Key」和第 5 条中「首次安装时规划全量回补」。
- 相关文档：[首次登录引导](../interfaces/rest_api.md#setup)、[配置](../interfaces/configuration.md#settings)、[在服务器上安装](../operations/runbook.md#server_install)、[前端](../architecture/frontend.md#pages)。

## 背景

同花顺金融数据服务的 API Key 原来在部署时提供：写进环境变量 `HITHINK_FINANCE_API_KEY`，`install.sh` 询问后写入 `deploy/.env`。用户要求：Key 在第一次打开网页时，由引导页让使用者自己配置，并告诉他在哪里生成（<https://fuyao.aicubes.cn/admin/>）；其他配置仍在启动时通过 `.env` 提供。

这样部署的人不需要拿到使用者的 Key，Key 也不必出现在服务器的配置文件和 shell 历史里。

## 决策

1. `HITHINK_FINANCE_API_KEY` 改为可选。Key 的来源按优先级：环境变量 → 网页中保存的值（`app.setting` 表，`name = 'hithink_api_key'`）。环境变量设置时，网页上只显示、不能修改。
2. 两者都没有时，服务照常启动：执行循环不认领任务，定时调度不发起增量，日志提示去网页配置；`niulai work --until-idle` 直接报错退出。前端所有页面都跳转到引导页 `/setup`。
3. 保存前先用这个 Key 请求一次接口（交易日历），被拒绝时返回 400，连不上时返回 503，都不保存。保存后立即生效，不需要重启。
4. 第一次配置成功、数据库里还没有任何回补运行时，自动规划 P0、P1 的全量回补；以后更换 Key 不再规划。
5. 接口和页面只显示 Key 的末尾 4 位。

## 取舍

- **数据库明文保存与加密保存**：加密需要另一把密钥，这把密钥仍要放在环境变量或文件里，等于把问题移到别处。能读 `app.setting` 的人也能读全部数据，数据库的访问控制就是它的保护边界；页面和接口不回显完整 Key，日志也不记录。
- **谁能修改 Key**：程序没有用户体系，能打开网页的人都能更换 Key。对公网开放时页面在 Caddy 登录之后，见 [ADR：部署方案](deployment.md)；只监听本机时只有能登录服务器的人能访问。
- **保留环境变量**：本机开发和自动化部署仍可以直接用环境变量，不需要打开网页。代价是两个来源并存，所以规定环境变量优先，并在设置页标明来源。

## 后果

- 新服务器安装后，要有人打开网页配置 Key，同步才会开始；在此之前页面能打开，但没有数据。
- Key 失效（被删除或停用）后，任务会以 2001 等错误码失败，到设置页更换 Key 即可，见 [处理失败任务](../operations/runbook.md#failure_handling)。
- 同一个 Key 仍然不要同时用在两套部署上。
