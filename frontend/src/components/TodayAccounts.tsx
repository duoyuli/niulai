import { clsx } from "clsx";
import { Link } from "react-router";
import { useListAccounts } from "../api/generated/accounts/accounts";
import type { AccountSummary } from "../api/generated/model";
import { yuan } from "../lib/accounts";
import { dayLabel, upDownClass } from "../lib/format";
import { Card, Spinner } from "./ui";

const HOME_ACCOUNT_LIMIT = 3;

function adviceLine(a: AccountSummary) {
  const b = a.advice;
  if (!b) return <span className="text-zinc-400">等待第一份建议（数据同步、巡检完成后自动生成）</span>;
  const act = b.sells + b.reduces + b.buys > 0;
  return (
    <span className={act ? "font-medium text-zinc-900 dark:text-zinc-100" : "text-zinc-500"}>
      {dayLabel(b.next_date)}运营建议：{b.headline}
      {act && b.pending > 0 && (
        <span className="ml-1 text-xs font-normal text-amber-700 dark:text-amber-400">（{b.pending} 条待处理）</span>
      )}
    </span>
  );
}

/** 首页上的「我的账户」：每个账户一行。单独成组件，避免首页静态拉入账户列表页的代码分包。 */
export function TodayAccounts() {
  const list = useListAccounts();
  return (
    <Card
      title="我的账户"
      extra={
        <Link to="/accounts" className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
          全部账户 →
        </Link>
      }
    >
      {list.isPending ? (
        <Spinner />
      ) : !list.data || list.data.length === 0 ? (
        <p className="text-sm text-zinc-500">
          <Link to="/accounts/new" className="text-accent hover:underline">
            建一个账户
          </Link>
          ，每天收到按你的持仓生成的调仓建议。
        </p>
      ) : (
        <>
          <ul className="flex flex-col divide-y divide-zinc-100 dark:divide-zinc-800">
            {list.data.slice(0, HOME_ACCOUNT_LIMIT).map((a) => (
              <li key={a.account.account_id}>
                <Link
                  to={`/accounts/${a.account.account_id}`}
                  className="flex flex-wrap items-center gap-x-4 gap-y-1 py-2 text-sm hover:opacity-80"
                >
                  <span className="w-32 truncate font-medium">{a.account.name}</span>
                  <span className="num w-28 text-zinc-500">{yuan(a.total_assets)}</span>
                  <span className={clsx("num w-24", upDownClass(a.day_pnl))}>{yuan(a.day_pnl)}</span>
                  {adviceLine(a)}
                </Link>
              </li>
            ))}
          </ul>
          {list.data.length > HOME_ACCOUNT_LIMIT && (
            <div className="mt-2 border-t border-zinc-100 pt-2 text-center dark:border-zinc-800">
              <Link to="/accounts" className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
                查看更多账户（还有 {list.data.length - HOME_ACCOUNT_LIMIT} 个）→
              </Link>
            </div>
          )}
        </>
      )}
    </Card>
  );
}
