import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import type { StrategyItem } from "../api/generated/model";
import {
  getListStrategiesQueryKey,
  useListStrategies,
  useReorderStrategies,
  useRunScreening,
} from "../api/generated/strategy/strategy";
import { StrategyNav } from "../components/StrategyNav";
import { Badge, buttonClass, Card, Empty, errorText, PageHeader, QueryView } from "../components/ui";
import { useIsAdmin } from "../hooks/useIsAdmin";
import { fmtInt } from "../lib/format";
import { invalidateStrategyQueries } from "../lib/queryInvalidation";
import { holdingsText } from "../lib/strategy";

export function StrategiesPage() {
  const qc = useQueryClient();
  const list = useListStrategies();
  const admin = useIsAdmin();
  const [sortOpen, setSortOpen] = useState(false);
  const run = useRunScreening({
    // 巡检结果出现在策略列表、策略详情和个股诊断里，全部刷新
    mutation: { onSuccess: () => invalidateStrategyQueries(qc) },
  });

  return (
    <>
      <StrategyNav />
      <PageHeader
        title="策略"
        desc="策略 = 买入条件 + 卖出规则，例如「5 日均线上穿 20 日均线时买，跌破 20 日均线或亏损 8% 时卖」。每天收盘数据同步完后在全市场算一遍，给出最新买点、最新卖点和持有中的股票；也可以看它过去几年的表现。条件由信号和指标组成，在上面的「信号」「指标」里能看到每一个是什么。"
        extra={
          admin && (
            <div className="flex w-full items-center justify-between gap-2">
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
              <button type="button" className={buttonClass("ghost")} onClick={() => setSortOpen(true)}>
                排序
              </button>
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
          {(items) => (
            <StrategyGrid items={items} admin={admin} sortOpen={sortOpen} onCloseSort={() => setSortOpen(false)} />
          )}
        </QueryView>
      </Card>
    </>
  );
}

function StrategyGrid({
  items,
  admin,
  sortOpen,
  onCloseSort,
}: {
  items: StrategyItem[];
  admin: boolean;
  sortOpen: boolean;
  onCloseSort: () => void;
}) {
  const qc = useQueryClient();
  const [draft, setDraft] = useState<StrategyItem[]>(items);
  const [draggingId, setDraggingId] = useState<number | null>(null);
  const lastTargetRef = useRef<number | null>(null);
  const reorder = useReorderStrategies({
    mutation: {
      onSuccess: () => qc.invalidateQueries({ queryKey: getListStrategiesQueryKey() }),
    },
  });
  useEffect(() => {
    if (sortOpen) {
      setDraft(items);
      setDraggingId(null);
      lastTargetRef.current = null;
    }
  }, [sortOpen, items]);
  const moveBefore = (targetId: number) => {
    if (draggingId === null || draggingId === targetId || lastTargetRef.current === targetId) return;
    const from = draft.findIndex((s) => s.strategy_id === draggingId);
    const to = draft.findIndex((s) => s.strategy_id === targetId);
    if (from < 0 || to < 0) return;
    const next = [...draft];
    const [item] = next.splice(from, 1);
    if (item) next.splice(from < to ? to - 1 : to, 0, item);
    setDraft(next);
    lastTargetRef.current = targetId;
  };
  const saveOrder = () => {
    if (draft.every((s, i) => s.strategy_id === items[i]?.strategy_id)) {
      onCloseSort();
      return;
    }
    reorder.mutate({ data: { strategy_ids: draft.map((s) => s.strategy_id) } }, { onSuccess: onCloseSort });
  };
  const closeSort = () => {
    if (!reorder.isPending) onCloseSort();
  };
  if (items.length === 0)
    return <Empty>{admin ? "还没有策略。点「新建策略」，从模板开始改最简单。" : "还没有策略，请管理员添加。"}</Empty>;
  return (
    <>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {items.map((s) => (
          <StrategyCard key={s.strategy_id} s={s} />
        ))}
      </div>
      {sortOpen && (
        <StrategySortDialog
          items={draft}
          draggingId={draggingId}
          saving={reorder.isPending}
          error={reorder.error}
          onClose={closeSort}
          onSave={saveOrder}
          onDragStart={(id) => {
            setDraggingId(id);
            lastTargetRef.current = null;
          }}
          onDragOver={moveBefore}
          onDragEnd={() => {
            setDraggingId(null);
            lastTargetRef.current = null;
          }}
        />
      )}
    </>
  );
}

function StrategyCard({ s }: { s: StrategyItem }) {
  const latest = s.latest;
  return (
    <div className="relative">
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
    </div>
  );
}

function StrategySortDialog({
  items,
  draggingId,
  saving,
  error,
  onClose,
  onSave,
  onDragStart,
  onDragOver,
  onDragEnd,
}: {
  items: StrategyItem[];
  draggingId: number | null;
  saving: boolean;
  error: unknown;
  onClose: () => void;
  onSave: () => void;
  onDragStart: (id: number) => void;
  onDragOver: (id: number) => void;
  onDragEnd: () => void;
}) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" role="presentation">
      <div
        className="flex max-h-[min(720px,calc(100vh-2rem))] w-full max-w-xl flex-col rounded-xl border border-zinc-200 bg-white shadow-xl dark:border-zinc-700 dark:bg-zinc-900"
        role="dialog"
        aria-modal="true"
        aria-labelledby="strategy-sort-title"
      >
        <div className="flex items-center justify-between border-b border-zinc-100 px-5 py-4 dark:border-zinc-800">
          <div>
            <h2 id="strategy-sort-title" className="font-semibold">
              调整策略顺序
            </h2>
            <p className="mt-1 text-xs text-zinc-500">拖动左侧手柄调整顺序，保存后才会生效。</p>
          </div>
          <button
            type="button"
            className="text-xl leading-none text-zinc-400 hover:text-zinc-700 dark:hover:text-zinc-200"
            onClick={onClose}
            aria-label="关闭"
          >
            ×
          </button>
        </div>
        <div className="overflow-y-auto p-4">
          <ol className="space-y-2">
            {items.map((item, index) => (
              <li
                key={item.strategy_id}
                draggable={!saving}
                onDragStart={(event) => {
                  event.dataTransfer.effectAllowed = "move";
                  event.dataTransfer.setData("text/plain", String(item.strategy_id));
                  onDragStart(item.strategy_id);
                }}
                onDragOver={(event) => {
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "move";
                  onDragOver(item.strategy_id);
                }}
                onDragEnd={onDragEnd}
                className={`flex items-center gap-3 rounded-lg border px-3 py-3 transition-colors ${draggingId === item.strategy_id ? "border-accent/60 bg-accent/10 opacity-50" : "border-zinc-200 bg-zinc-50 dark:border-zinc-700 dark:bg-zinc-800/50"}`}
              >
                <span
                  className="cursor-grab text-lg leading-none text-zinc-400 active:cursor-grabbing"
                  aria-hidden="true"
                >
                  ⠿
                </span>
                <span className="w-6 text-right text-xs text-zinc-400">{index + 1}</span>
                <span className="min-w-0 flex-1 truncate text-sm font-medium">{item.name}</span>
                {item.builtin && <Badge tone="ok">内置</Badge>}
              </li>
            ))}
          </ol>
          {!!error && <p className="mt-3 text-sm text-red-600">顺序保存失败：{errorText(error)}</p>}
        </div>
        <div className="flex justify-end gap-2 border-t border-zinc-100 px-5 py-3 dark:border-zinc-800">
          <button type="button" className={buttonClass("ghost")} disabled={saving} onClick={onClose}>
            取消
          </button>
          <button type="button" className={buttonClass()} disabled={saving} onClick={onSave}>
            {saving ? "保存中…" : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}
