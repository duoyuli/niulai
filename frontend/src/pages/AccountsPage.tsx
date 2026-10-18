import { clsx } from "clsx";
import { Link } from "react-router";
import { useListAccounts } from "../api/generated/accounts/accounts";
import type { AccountSummary } from "../api/generated/model";
import { buttonClass, Card, Empty, PageHeader, QueryView, Spinner } from "../components/ui";
import { yuan } from "../lib/accounts";
import { dayLabel, upDownClass } from "../lib/format";

/** 建议的一句话，带颜色：要操作的突出显示 */
export function AdviceLine({ a }: { a: AccountSummary }) {
  const b = a.advice;
  if (!b) return <span className="text-zinc-400">等待第一份建议（数据同步、巡检完成后自动生成）</span>;
  const act = b.sells + b.reduces + b.buys > 0;
  return (
    <span className={act ? "font-medium text-zinc-900 dark:text-zinc-100" : "text-zinc-500"}>
      {dayLabel(b.next_date)}操作计划：{b.headline}
      {act && b.pending > 0 && (
        <span className="ml-1 text-xs font-normal text-amber-700 dark:text-amber-400">（{b.pending} 条待处理）</span>
      )}
    </span>
  );
}

function AccountCard({ a }: { a: AccountSummary }) {
  return (
    <Link
      to={`/accounts/${a.account.account_id}`}
      className="flex flex-col gap-2 rounded-xl border border-zinc-200 bg-white p-4 transition hover:border-zinc-400 dark:border-zinc-800 dark:bg-zinc-900"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-semibold">{a.account.name}</span>
      </div>
      <div className="flex flex-wrap gap-x-4 gap-y-1 text-sm">
        <span>
          总资产 <span className="num font-medium">{yuan(a.total_assets)}</span>
        </span>
        <span className={clsx("num", upDownClass(a.day_pnl))}>当日 {yuan(a.day_pnl)}</span>
        <span className="text-zinc-500">
          现金 {yuan(a.account.cash)} · 持仓 {a.positions} 只
        </span>
      </div>
      <p className="text-sm">
        <AdviceLine a={a} />
      </p>
      {a.check_hint && <p className="text-xs text-amber-700 dark:text-amber-400">{a.check_hint}</p>}
    </Link>
  );
}

/** 我的账户：每个账户一张卡片，写着下个交易日要做什么 */
export function AccountsPage() {
  const list = useListAccounts();
  return (
    <>
      <PageHeader
        title="我的账户"
        desc="每个证券账户建一个。每天收盘、数据同步完之后，系统从全部策略选出的股票里，按过去表现好的策略优先，结合每个账户的持仓和风控规则，生成一份「明天怎么操作」，每条都写清楚为什么。照着做完点「已执行」，持仓自动更新。账户只有你自己看得到。"
        extra={
          <Link to="/accounts/new" className={buttonClass()}>
            新建账户
          </Link>
        }
      />
      <QueryView query={list}>
        {(items) =>
          items.length === 0 ? (
            <Card>
              <Empty>
                还没有账户。
                <Link to="/accounts/new" className="text-accent hover:underline">
                  建一个
                </Link>
                ，两步就好：起个名字、填现金和持仓。
              </Empty>
            </Card>
          ) : (
            <div className="grid gap-3 md:grid-cols-2">
              {items.map((a) => (
                <AccountCard key={a.account.account_id} a={a} />
              ))}
            </div>
          )
        }
      </QueryView>
    </>
  );
}

/** 今日页上的「我的账户」：每个账户一行 */
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
        <ul className="flex flex-col divide-y divide-zinc-100 dark:divide-zinc-800">
          {list.data.map((a) => (
            <li key={a.account.account_id}>
              <Link
                to={`/accounts/${a.account.account_id}`}
                className="flex flex-wrap items-center gap-x-4 gap-y-1 py-2 text-sm hover:opacity-80"
              >
                <span className="w-32 truncate font-medium">{a.account.name}</span>
                <span className="num w-28 text-zinc-500">{yuan(a.total_assets)}</span>
                <span className={clsx("num w-24", upDownClass(a.day_pnl))}>{yuan(a.day_pnl)}</span>
                <AdviceLine a={a} />
              </Link>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
