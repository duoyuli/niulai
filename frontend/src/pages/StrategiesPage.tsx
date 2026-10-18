import { useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router";
import type { StrategyItem } from "../api/generated/model";
import { useListStrategies, useRunScreening } from "../api/generated/strategy/strategy";
import { StrategyNav } from "../components/StrategyNav";
import { Badge, buttonClass, Card, Empty, errorText, PageHeader, QueryView } from "../components/ui";
import { useIsAdmin } from "../hooks/useIsAdmin";
import { fmtInt } from "../lib/format";
import { holdingsText } from "../lib/strategy";

export function StrategiesPage() {
  const qc = useQueryClient();
  const list = useListStrategies();
  const admin = useIsAdmin();
  const run = useRunScreening({
    // 巡检结果出现在策略列表、策略详情和个股诊断里，全部刷新
    mutation: { onSuccess: () => qc.invalidateQueries() },
  });

  return (
    <>
      <StrategyNav />
      <PageHeader
        title="策略"
        desc="策略 = 买入条件 + 卖出规则，例如「5 日均线上穿 20 日均线时买，跌破 20 日均线或亏损 8% 时卖」。每天收盘数据同步完后在全市场算一遍，给出最新买点、最新卖点和持有中的股票；也可以看它过去几年的表现。条件由信号和指标组成，在上面的「信号」「指标」里能看到每一个是什么。"
        extra={
          admin && (
            <div className="flex items-center gap-2">
              <button
                type="button"
                className={buttonClass("ghost")}
                disabled={run.isPending}
                title="现在就用开启巡检的策略在最新交易日选一次股，结果会覆盖当天已有的结果"
                onClick={() => run.mutate({})}
              >
                {run.isPending ? "巡检中…" : "立即巡检"}
              </button>
              <Link to="/strategies/new" className={buttonClass()}>
                新建策略
              </Link>
            </div>
          )
        }
      />
      {run.data && (
        <p className="text-sm text-zinc-500">
          {run.data.trade_date
            ? `已在 ${run.data.trade_date} 用 ${run.data.strategies} 个策略检查了 ${fmtInt(run.data.scanned)} 只股票，用时 ${(run.data.ms / 1000).toFixed(1)} 秒。`
            : "没有开启巡检的策略，或本地还没有日线。"}
        </p>
      )}
      {!!run.error && <p className="text-sm text-red-600">{errorText(run.error)}</p>}

      <Card>
        <QueryView query={list}>
          {(items) =>
            items.length === 0 ? (
              <Empty>{admin ? "还没有策略。点「新建策略」，从模板开始改最简单。" : "还没有策略，请管理员添加。"}</Empty>
            ) : (
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
                {items.map((s) => (
                  <StrategyCard key={s.strategy_id} s={s} />
                ))}
              </div>
            )
          }
        </QueryView>
      </Card>
    </>
  );
}

function StrategyCard({ s }: { s: StrategyItem }) {
  const latest = s.latest;
  return (
    <Link
      to={`/strategies/${s.strategy_id}`}
      className="flex flex-col gap-2 rounded-lg border border-zinc-200 p-3 transition hover:border-accent/60 hover:bg-zinc-50 dark:border-zinc-800 dark:hover:bg-zinc-800/50"
    >
      <div className="flex items-start justify-between gap-2">
        <span className="font-medium">{s.name}</span>
        <span className="flex shrink-0 gap-1">
          {s.builtin && <Badge tone="ok">内置</Badge>}
          {!s.show_on_today && <Badge>不在首页</Badge>}
          {s.screening ? <Badge tone="info">每日巡检</Badge> : <Badge>已暂停</Badge>}
        </span>
      </div>
      <p className="line-clamp-2 text-xs text-zinc-500">{s.description || s.explained.all.join("，并且")}</p>
      <div className="mt-auto flex items-baseline gap-2 text-sm">
        {latest ? (
          <>
            <span className="text-xs text-zinc-500">买点</span>
            <span className="num text-lg font-semibold text-up">{fmtInt(latest.matched)}</span>
            <span className="text-xs text-zinc-500">卖点</span>
            <span className="num text-lg font-semibold text-down">{fmtInt(latest.sells)}</span>
            <span className="text-xs text-zinc-500" title="按规则推算的持有中股票数">
              持有 {holdingsText(latest.holdings)}
            </span>
            <span className="num ml-auto text-xs text-zinc-400">{latest.trade_date}</span>
          </>
        ) : (
          <span className="text-xs text-zinc-400">还没有巡检过</span>
        )}
      </div>
    </Link>
  );
}
