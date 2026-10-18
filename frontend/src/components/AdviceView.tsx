import { useQueryClient } from "@tanstack/react-query";
import { clsx } from "clsx";
import { Fragment, useState } from "react";
import { Link } from "react-router";
import {
  getAdviceHistoryQueryKey,
  getCurrentAdviceQueryKey,
  getGetAccountQueryKey,
  getListAccountsQueryKey,
  getListTradesQueryKey,
  useClearItem,
  useCurrentAdvice,
  useExecuteItem,
  useRefreshAdvice,
  useSkipItem,
} from "../api/generated/accounts/accounts";
import type { AdviceItem, AdviceResponse, StoredAdvice } from "../api/generated/model";
import { ACTIONS, NOTE_KINDS, yuan } from "../lib/accounts";
import { dayLabel, fmtDateTime, fmtPrice } from "../lib/format";
import { buttonClass, Card, Empty, errorText, Field, inputClass, QueryView, Table } from "./ui";

/** 一个账户某一天的调仓建议。date 为空时是最新一份 */
export function AdviceView({ accountId, date }: { accountId: number; date?: string }) {
  const query = useCurrentAdvice(accountId, date ? { date } : undefined, {
    query: { refetchInterval: (q) => (q.state.data?.waiting ? 30_000 : false) },
  });
  const qc = useQueryClient();
  const refresh = useRefreshAdvice({
    mutation: { onSuccess: (r) => qc.setQueryData(getCurrentAdviceQueryKey(accountId), r) },
  });
  return (
    <QueryView query={query}>
      {(r) =>
        !r.advice ? (
          <Card>
            <Empty>{r.waiting ?? "还没有建议"}</Empty>
          </Card>
        ) : (
          <Body
            accountId={accountId}
            s={r.advice}
            latestBar={r.latest_bar}
            readonly={!!date}
            refreshing={refresh.isPending}
            onRefresh={() => refresh.mutate({ accountId })}
          />
        )
      }
    </QueryView>
  );
}

function Body({
  accountId,
  s,
  latestBar,
  readonly,
  refreshing,
  onRefresh,
}: {
  accountId: number;
  s: StoredAdvice;
  latestBar?: string | null;
  readonly: boolean;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  const a = s.advice;
  const sells = a.items.filter((i) => i.action === "sell" || i.action === "reduce");
  const buys = a.items.filter((i) => i.action === "buy");
  const rest = a.items.filter((i) => i.action === "hold" || i.action === "wait");
  const pending = [...sells, ...buys].filter((i) => !s.states[i.key]).length;
  return (
    <div className="flex flex-col gap-4">
      <Card>
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="text-xl font-semibold">
              {dayLabel(a.next_date)}操作计划：{a.headline}
            </h2>
            <p className="mt-1 text-xs text-zinc-500">
              根据 {a.trade_date} 收盘的数据生成，下一个交易日开盘后照着做 · 生成于 {fmtDateTime(s.created_at)}
            </p>
            {a.why_nothing && <p className="mt-2 text-sm text-zinc-600 dark:text-zinc-400">{a.why_nothing}</p>}
            {sells.length + buys.length > 0 && !readonly && (
              <p className="mt-2 text-sm text-zinc-500">
                {pending > 0
                  ? `还有 ${pending} 条没处理：在券商做完后点「已执行」，持仓和现金会自动更新。`
                  : "都处理完了。"}
              </p>
            )}
            {a.market && <p className="mt-2 text-xs text-zinc-500">市场：{a.market}</p>}
          </div>
          {!readonly && (
            <button
              type="button"
              className={buttonClass("ghost")}
              disabled={refreshing}
              onClick={onRefresh}
              title="按现在的持仓和最新的巡检结果重新生成"
            >
              {refreshing ? "生成中…" : "重新生成"}
            </button>
          )}
        </div>
        {a.settings_hint && (
          <p className="mt-3 rounded-lg bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:bg-amber-500/10 dark:text-amber-300">
            {a.settings_hint}。
            {!readonly && (
              <Link to="?tab=settings" className="ml-1 underline">
                去设置
              </Link>
            )}
          </p>
        )}
        {latestBar && latestBar > a.trade_date && (
          <p className="mt-2 text-xs text-amber-700 dark:text-amber-400">
            已经有 {latestBar} 的日线，新的计划等巡检完成后自动生成。
          </p>
        )}
        {a.notes.map((n) => (
          <p
            key={n}
            className="mt-2 rounded-md bg-zinc-100 px-2 py-1 text-xs text-zinc-600 dark:bg-zinc-800 dark:text-zinc-400"
          >
            {n}
          </p>
        ))}
      </Card>

      {sells.length > 0 && (
        <Card title={`卖出（${dayLabel(a.next_date)}开盘后）`}>
          <OrderTable accountId={accountId} s={s} items={sells} readonly={readonly} kind="sell" />
          <p className="mt-2 text-xs text-zinc-500">开盘一字跌停卖不出时，下一份计划会继续提示。</p>
        </Card>
      )}

      {buys.length > 0 && (
        <Card title={`买入（${dayLabel(a.next_date)}开盘后，开盘价不超过上限才买）`}>
          <OrderTable accountId={accountId} s={s} items={buys} readonly={readonly} kind="buy" />
          <p className="mt-2 text-xs text-zinc-500">
            开盘价高于「最高买入价」就是追高，这只放弃；开盘一字涨停买不进，这条作废。也可以直接按最高买入价挂限价单。买入后跌到止损价，计划会提示卖出。
          </p>
        </Card>
      )}

      {rest.length > 0 && (
        <Card title={`继续拿（${rest.length} 只）`}>
          <div className="flex flex-col gap-2">
            {rest.map((i) => (
              <ItemCard key={i.key} item={i} />
            ))}
          </div>
        </Card>
      )}

      <div className="grid gap-4 lg:grid-cols-2">
        <Card title="照着做完之后">
          <p className="text-sm">
            现金约 <span className="num font-medium">{yuan(a.after.cash)}</span>（占 {a.after.cash_pct.toFixed(0)}
            %），持有 {a.after.positions} 只
          </p>
          {a.after.weights.length > 0 && (
            <ul className="mt-2 space-y-1 text-sm">
              {a.after.weights.map((w) => (
                <li key={w.thscode} className="flex items-center gap-2">
                  <span className="w-28 truncate">{w.name}</span>
                  <span
                    className="h-2 rounded bg-accent/60"
                    style={{ width: `${Math.min(w.weight_pct, 100) * 2}px` }}
                  />
                  <span className="num text-xs text-zinc-500">{w.weight_pct.toFixed(1)}%</span>
                </li>
              ))}
            </ul>
          )}
          <p className="mt-2 text-xs text-zinc-500">
            按 {a.trade_date} 收盘价估算，总资产 {yuan(a.total_assets)}。
          </p>
        </Card>
        <Card title={`${a.trade_date} 收盘出现买点、但没选上的`}>
          {a.skipped.length === 0 ? (
            <p className="text-sm text-zinc-400">没有</p>
          ) : (
            <ul className="space-y-1 text-sm">
              {a.skipped.map((k) => (
                <li key={k.thscode}>
                  <Link to={`/securities/${k.thscode}`} className="hover:underline">
                    {k.name}
                  </Link>
                  <span className="text-zinc-500">：{k.reason}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>
      </div>
      <p className="text-xs text-zinc-400">
        计划只根据规则和已经发生的数据，不预测涨跌，也不保证收益。参考价是 {a.trade_date} 的收盘价，实际成交以券商为准。
      </p>
    </div>
  );
}

/** 下单表：卖出或买入的每一行，点「为什么」展开理由；已执行、不做在每行右边 */
function OrderTable({
  accountId,
  s,
  items,
  readonly,
  kind,
}: {
  accountId: number;
  s: StoredAdvice;
  items: AdviceItem[];
  readonly: boolean;
  kind: "sell" | "buy";
}) {
  const [open, setOpen] = useState<string | null>(null);
  const [executing, setExecuting] = useState<string | null>(null);
  const trade = s.advice.trade_date.slice(5);
  const cols = kind === "buy" ? 7 : 6;
  return (
    <Table>
      <thead>
        <tr>
          <th>股票</th>
          <th>{kind === "buy" ? "买多少" : "卖多少"}</th>
          <th>参考价（{trade} 收盘）</th>
          {kind === "buy" && <th>最高买入价（开盘不超过）</th>}
          {kind === "buy" ? <th>止损价</th> : <th>为什么卖</th>}
          <th>约多少钱</th>
          <th />
        </tr>
      </thead>
      <tbody>
        {items.map((i) => {
          const state = s.states[i.key];
          const sellAll = i.action === "sell";
          return (
            <Fragment key={i.key}>
              <tr className={clsx(state && "opacity-60")}>
                <td className="max-w-72">
                  <Link to={`/securities/${i.thscode}`} className="font-medium hover:underline">
                    {i.name}
                  </Link>
                  <span className="ml-1 text-xs text-zinc-400">{i.thscode}</span>
                  {kind === "buy" && (
                    <span className="block truncate text-xs text-zinc-500" title={i.headline}>
                      {i.headline}
                    </span>
                  )}
                </td>
                <td className="num">
                  {i.shares} 股
                  {kind === "sell" && <span className="block text-xs text-zinc-500">{sellAll ? "全部" : "减仓"}</span>}
                </td>
                <td className="num">{fmtPrice(i.ref_price)}</td>
                {kind === "buy" && (
                  <td className="num font-medium">
                    {i.limit_price ? fmtPrice(i.limit_price) : "—"}
                    {i.limit_price && (
                      <span className="block text-xs font-normal text-zinc-500">
                        高 {((i.limit_price / i.ref_price - 1) * 100).toFixed(0)}%
                      </span>
                    )}
                  </td>
                )}
                {kind === "buy" ? (
                  <td className="num">{i.stop_price ? fmtPrice(i.stop_price) : "—"}</td>
                ) : (
                  <td className="max-w-72 text-xs text-zinc-600 dark:text-zinc-400">{i.headline}</td>
                )}
                <td className="num">{yuan(i.amount)}</td>
                <td className="whitespace-nowrap text-right text-sm">
                  {state ? (
                    <span className="text-xs text-zinc-500">{state === "executed" ? "✓ 已执行" : "不做"}</span>
                  ) : (
                    !readonly && (
                      <span className="inline-flex items-center gap-3">
                        <button
                          type="button"
                          className={buttonClass()}
                          onClick={() => setExecuting(executing === i.key ? null : i.key)}
                        >
                          已执行
                        </button>
                        <SkipButton accountId={accountId} adviceId={s.advice_id} itemKey={i.key} />
                      </span>
                    )
                  )}
                  {state === "skipped" && !readonly && (
                    <span className="ml-2">
                      <ClearButton accountId={accountId} adviceId={s.advice_id} itemKey={i.key} />
                    </span>
                  )}
                  <button
                    type="button"
                    className="ml-3 text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
                    onClick={() => setOpen(open === i.key ? null : i.key)}
                  >
                    {open === i.key ? "收起" : "为什么"}
                  </button>
                </td>
              </tr>
              {executing === i.key && (
                <tr>
                  <td colSpan={cols}>
                    <ExecuteForm accountId={accountId} s={s} item={i} onClose={() => setExecuting(null)} />
                  </td>
                </tr>
              )}
              {open === i.key && (
                <tr>
                  <td colSpan={cols} className="bg-zinc-50 dark:bg-zinc-800/40">
                    <ItemDetail item={i} />
                  </td>
                </tr>
              )}
            </Fragment>
          );
        })}
      </tbody>
    </Table>
  );
}

/** 一条建议的理由：规则、支持的数字、需要注意、怎么做，以及 K 线链接 */
function ItemDetail({ item }: { item: AdviceItem }) {
  const trade = item.action === "sell" || item.action === "reduce" || item.action === "buy";
  const chart = item.strategy_id
    ? `/securities/${item.thscode}?overlay=strategy:${item.strategy_id},ind:ma:20,ind:ma:60`
    : `/securities/${item.thscode}`;
  return (
    <div className="flex flex-col gap-3 py-1 text-sm">
      <section>
        <h4 className="text-xs font-medium text-zinc-500">为什么</h4>
        <p className="mt-1">{item.rule}</p>
        <ul className="mt-1 space-y-0.5">
          {item.evidence.map((n) => (
            <li key={n.text}>
              <span className="mr-1 text-xs text-zinc-400">{NOTE_KINDS[n.kind] ?? n.kind}</span>
              {n.text}
            </li>
          ))}
        </ul>
      </section>
      {item.cautions.length > 0 && (
        <section>
          <h4 className="text-xs font-medium text-amber-700 dark:text-amber-400">需要注意</h4>
          <ul className="mt-1 space-y-0.5">
            {item.cautions.map((n) => (
              <li key={n.text}>{n.text}</li>
            ))}
          </ul>
        </section>
      )}
      {item.plan.length > 0 && (
        <section>
          <h4 className="text-xs font-medium text-zinc-500">{trade ? "怎么做" : "什么时候会变"}</h4>
          <ul className="mt-1 list-disc space-y-0.5 pl-4">
            {item.plan.map((p) => (
              <li key={p}>{p}</li>
            ))}
          </ul>
        </section>
      )}
      <Link to={chart} className="self-start text-accent hover:underline">
        看 K 线{item.strategy_id ? "和策略买卖点" : ""} →
      </Link>
    </div>
  );
}

/** 继续拿、不动的持仓：一行结论，点开看理由 */
function ItemCard({ item }: { item: AdviceItem }) {
  const [open, setOpen] = useState(false);
  const info = ACTIONS[item.action];
  return (
    <div className="rounded-lg border border-zinc-200 dark:border-zinc-800">
      <button
        type="button"
        className="flex w-full flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2 text-left"
        onClick={() => setOpen(!open)}
      >
        <span className={clsx("rounded-md px-2 py-0.5 text-xs font-medium", info.chip)}>{info.label}</span>
        <span className="font-medium">{item.name}</span>
        <span className="text-xs text-zinc-400">{item.thscode}</span>
        <span className="min-w-0 basis-full text-sm text-zinc-600 sm:basis-auto dark:text-zinc-400">
          {item.headline}
        </span>
        <span className="ml-auto text-xs text-zinc-400">{open ? "收起" : "为什么"}</span>
      </button>
      {open && (
        <div className="border-t border-zinc-100 px-3 py-2 dark:border-zinc-800">
          <ItemDetail item={item} />
        </div>
      )}
    </div>
  );
}

/** 处理完一条后：建议、账户、流水都要刷新 */
function useAfterChange(accountId: number) {
  const qc = useQueryClient();
  return (r: AdviceResponse) => {
    qc.setQueryData(getCurrentAdviceQueryKey(accountId), r);
    qc.invalidateQueries({ queryKey: getGetAccountQueryKey(accountId) });
    qc.invalidateQueries({ queryKey: getListAccountsQueryKey() });
    qc.invalidateQueries({ queryKey: getListTradesQueryKey(accountId) });
    qc.invalidateQueries({ queryKey: getAdviceHistoryQueryKey(accountId) });
  };
}

function ExecuteForm({
  accountId,
  s,
  item,
  onClose,
}: {
  accountId: number;
  s: StoredAdvice;
  item: AdviceItem;
  onClose: () => void;
}) {
  const done = useAfterChange(accountId);
  const [shares, setShares] = useState(String(item.shares));
  const [price, setPrice] = useState(fmtPrice(item.ref_price));
  const [fee, setFee] = useState("");
  const exec = useExecuteItem({
    mutation: {
      onSuccess: (r) => {
        done(r);
        onClose();
      },
    },
  });
  return (
    <div className="flex flex-wrap items-end gap-3 rounded-lg bg-zinc-50 p-3 dark:bg-zinc-800/50">
      <Field label="实际成交股数">
        <input
          className={`${inputClass} w-28`}
          inputMode="numeric"
          value={shares}
          onChange={(e) => setShares(e.target.value)}
        />
      </Field>
      <Field label="实际成交价">
        <input
          className={`${inputClass} w-28`}
          inputMode="decimal"
          value={price}
          onChange={(e) => setPrice(e.target.value)}
        />
      </Field>
      <Field label="费用（不填按费率估算）">
        <input
          className={`${inputClass} w-28`}
          inputMode="decimal"
          value={fee}
          onChange={(e) => setFee(e.target.value)}
        />
      </Field>
      <button
        type="button"
        className={buttonClass()}
        disabled={exec.isPending || !(Number(shares) > 0) || !(Number(price) > 0)}
        onClick={() =>
          exec.mutate({
            accountId,
            adviceId: s.advice_id,
            key: item.key,
            data: { shares: Math.round(Number(shares)), price: Number(price), fee: fee === "" ? null : Number(fee) },
          })
        }
      >
        {exec.isPending ? "记录中…" : "记下来"}
      </button>
      <button type="button" className="text-sm text-zinc-500" onClick={onClose}>
        取消
      </button>
      {!!exec.error && <p className="basis-full text-sm text-red-600">{errorText(exec.error)}</p>}
    </div>
  );
}

function SkipButton({ accountId, adviceId, itemKey }: { accountId: number; adviceId: number; itemKey: string }) {
  const done = useAfterChange(accountId);
  const skip = useSkipItem({ mutation: { onSuccess: done } });
  return (
    <button
      type="button"
      className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
      disabled={skip.isPending}
      onClick={() => skip.mutate({ accountId, adviceId, key: itemKey })}
    >
      不做
    </button>
  );
}

function ClearButton({ accountId, adviceId, itemKey }: { accountId: number; adviceId: number; itemKey: string }) {
  const done = useAfterChange(accountId);
  const clear = useClearItem({ mutation: { onSuccess: done } });
  return (
    <button
      type="button"
      className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
      disabled={clear.isPending}
      onClick={() => clear.mutate({ accountId, adviceId, key: itemKey })}
    >
      撤销「不做」
    </button>
  );
}
