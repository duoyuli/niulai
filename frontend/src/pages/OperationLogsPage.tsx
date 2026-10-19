import { useListOperationLogs } from "../api/generated/operation-log/operation-log";
import { Card, Empty, PageHeader, QueryView, Table } from "../components/ui";
import { fmtDateTime } from "../lib/format";

/** 管理员查看成功登录记录：用户、来源 IP、浏览器设备和时间。 */
export function OperationLogsPage() {
  const logs = useListOperationLogs({ limit: 200 }, { query: { staleTime: 30_000 } });
  return (
    <>
      <PageHeader title="操作日志" desc="目前只记录成功登录；登录失败不会写入。设备信息来自登录请求的 User-Agent。" />
      <Card title="最近登录">
        <QueryView query={logs}>
          {(items) =>
            items.length === 0 ? (
              <Empty>还没有登录记录</Empty>
            ) : (
              <Table>
                <thead>
                  <tr>
                    <th>时间</th>
                    <th>用户</th>
                    <th>IP 地址</th>
                    <th>设备</th>
                  </tr>
                </thead>
                <tbody>
                  {items.map((item) => (
                    <tr key={item.log_id}>
                      <td className="num whitespace-nowrap">{fmtDateTime(item.created_at)}</td>
                      <td>{item.username}</td>
                      <td className="font-mono text-xs">{item.ip_address}</td>
                      <td className="max-w-xl whitespace-normal break-words text-xs text-zinc-500">{item.device}</td>
                    </tr>
                  ))}
                </tbody>
              </Table>
            )
          }
        </QueryView>
      </Card>
    </>
  );
}
