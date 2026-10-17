import { clsx } from "clsx";
import { useState } from "react";
import { useStyles } from "../api/generated/accounts/accounts";
import type { Style, StyleInfo, StyleParams } from "../api/generated/model";
import { WEAK_MARKET } from "../lib/accounts";
import { fmtPctValue, fmtPercent, upDownClass } from "../lib/format";
import { QueryView } from "./ui";

/** 一个风格的规则，写成几句白话 */
export function styleRules(p: StyleParams): string[] {
  return [
    `最多同时持有 ${p.max_positions} 只，单只最多占总资产的 ${p.max_weight_pct}%${p.min_cash_pct > 0 ? `，至少留 ${p.min_cash_pct}% 现金` : ""}`,
    `比成本价亏 ${p.stop_loss_pct}% 就卖（止损），收盘跌破 ${p.trend_ma} 日均线也卖`,
    `全市场站上 20 日均线的股票不到 ${p.weak_threshold_pct}% 时算弱市：${WEAK_MARKET[p.weak_market]}`,
    `第二天开盘比参考价高 ${p.chase_limit_pct}% 以上就不追`,
    p.require_edge ? "只采纳过去 3 年表现好于「随便买」的策略" : "参考的策略都采纳，不看过去表现",
  ];
}

/** 风格回测的一句话 */
export function BacktestLine({ b }: { b: NonNullable<StyleInfo["backtest"]> }) {
  const s = b.summary;
  return (
    <p className="text-xs text-zinc-500">
      过去 3 年回测：
      <span className={clsx("num font-medium", upDownClass(s.total_return))}>{fmtPercent(s.total_return, 1)}</span>
      {s.annual_return !== null && s.annual_return !== undefined && <>（年化 {fmtPercent(s.annual_return, 1)}）</>}
      ，最大回撤 <span className="num">{fmtPercent(-s.max_drawdown, 1)}</span>
      {s.benchmark_return !== null && s.benchmark_return !== undefined && (
        <>
          ；同期{s.benchmark_name} <span className="num">{fmtPercent(s.benchmark_return, 1)}</span>
        </>
      )}
      。平均持有 {s.avg_positions.toFixed(1)} 只，{s.trades} 笔交易
      {s.win_rate !== null && s.win_rate !== undefined && <>，{(s.win_rate * 100).toFixed(0)}% 赚钱</>}。
    </p>
  );
}

/** 三张风格卡片：一句话、规则、参考的策略和它们的过去表现、风格回测。点卡片选择 */
export function StyleCards({ value, onChange }: { value: Style; onChange: (s: Style) => void }) {
  const styles = useStyles();
  const [open, setOpen] = useState<Style | null>(null);
  return (
    <QueryView query={styles}>
      {(list) => (
        <div className="grid gap-3 md:grid-cols-3">
          {list.map((s) => (
            <div
              key={s.style}
              className={clsx(
                "flex flex-col gap-2 rounded-xl border p-4 text-sm transition",
                value === s.style
                  ? "border-accent ring-2 ring-accent/30"
                  : "border-zinc-200 hover:border-zinc-400 dark:border-zinc-700",
              )}
            >
              <button
                type="button"
                className="flex flex-col items-start gap-1 text-left"
                onClick={() => onChange(s.style)}
              >
                <span className="flex items-center gap-2 text-base font-semibold">
                  <span
                    className={clsx(
                      "inline-block size-4 rounded-full border-2",
                      value === s.style ? "border-accent bg-accent" : "border-zinc-300",
                    )}
                  />
                  {s.name}
                </span>
                <span className="text-zinc-600 dark:text-zinc-400">{s.tagline}</span>
              </button>
              <ul className="list-disc space-y-0.5 pl-4 text-xs text-zinc-600 dark:text-zinc-400">
                {styleRules(s.params).map((r) => (
                  <li key={r}>{r}</li>
                ))}
              </ul>
              {s.backtest ? (
                <BacktestLine b={s.backtest} />
              ) : (
                <p className="text-xs text-zinc-400">过去 3 年的回测还在计算（每周算一次）</p>
              )}
              <button
                type="button"
                className="self-start text-xs text-accent hover:underline"
                onClick={() => setOpen(open === s.style ? null : s.style)}
              >
                {open === s.style ? "收起" : `参考哪 ${s.strategies.length} 个策略`}
              </button>
              {open === s.style && (
                <ul className="space-y-1.5 text-xs">
                  {s.strategies.map((t) => {
                    const st = t.stat;
                    const excess =
                      st?.after20_avg_pct != null && st.base20_avg_pct != null
                        ? st.after20_avg_pct - st.base20_avg_pct
                        : null;
                    return (
                      <li key={t.key}>
                        <span className="font-medium">{t.name}</span>
                        <span className="text-zinc-500">：{t.description}</span>
                        {st && excess !== null && (
                          <span className="block text-zinc-500">
                            过去 3 年 {st.signals} 次买点，20 个交易日后平均 {fmtPctValue(st.after20_avg_pct, 1)}
                            ，比随便买 <span className={upDownClass(excess)}>{fmtPctValue(excess, 1)}</span>
                          </span>
                        )}
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          ))}
        </div>
      )}
    </QueryView>
  );
}
