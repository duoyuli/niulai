import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import {
  getCurrentAdviceQueryKey,
  getGetAccountQueryKey,
  getGetEquityQueryKey,
  getListAccountsQueryKey,
  useCalibrate,
  useParsePositions,
} from "../api/generated/accounts/accounts";
import { useGetSecurity } from "../api/generated/market/market";
import type { AccountDetail, ParsedRow } from "../api/generated/model";
import { SecuritySearch } from "./SecuritySearch";
import { buttonClass, errorText, Field, inputClass } from "./ui";

interface Row {
  thscode: string;
  name: string;
  shares: string;
  cost: string;
  opened_on: string;
}

function fromDetail(d: AccountDetail): Row[] {
  return d.positions.map((p) => ({
    thscode: p.position.thscode,
    name: p.name,
    shares: String(p.position.shares),
    cost: p.position.cost_price ? String(p.position.cost_price) : "",
    opened_on: p.position.opened_on ?? "",
  }));
}

/** 搜索加入的股票只有代码，名称单独取一次 */
function NameOf({ code, fallback }: { code: string; fallback: string }) {
  const q = useGetSecurity(code, { query: { enabled: !fallback, staleTime: Number.POSITIVE_INFINITY } });
  return <>{fallback || q.data?.name || code}</>;
}

/** 校准持仓和现金：照着券商 App 填一遍（可以直接粘贴），保存后和原来不同的地方各记一条流水，建议重新生成 */
export function PositionsEditor({
  detail,
  onDone,
  saveLabel = "保存",
}: {
  detail: AccountDetail;
  onDone: () => void;
  saveLabel?: string;
}) {
  const id = detail.summary.account.account_id;
  const qc = useQueryClient();
  const [cash, setCash] = useState(String(detail.summary.account.cash));
  const [rows, setRows] = useState<Row[]>(() => fromDetail(detail));
  const [text, setText] = useState("");
  const [parsed, setParsed] = useState<ParsedRow[] | null>(null);
  const parse = useParsePositions({ mutation: { onSuccess: setParsed } });
  const save = useCalibrate({
    mutation: {
      onSuccess: (d) => {
        qc.setQueryData(getGetAccountQueryKey(id), d);
        qc.invalidateQueries({ queryKey: getListAccountsQueryKey() });
        qc.invalidateQueries({ queryKey: getCurrentAdviceQueryKey(id) });
        qc.invalidateQueries({ queryKey: getGetEquityQueryKey(id) });
        onDone();
      },
    },
  });
  const set = (i: number, patch: Partial<Row>) => setRows((r) => r.map((x, k) => (k === i ? { ...x, ...patch } : x)));
  const add = (code: string, name = "", shares = "", cost = "") =>
    setRows((r) => {
      const i = r.findIndex((x) => x.thscode === code);
      if (i >= 0) return r.map((x, k) => (k === i ? { ...x, shares: shares || x.shares, cost: cost || x.cost } : x));
      return [...r, { thscode: code, name, shares, cost, opened_on: "" }];
    });
  const usable = (parsed ?? []).filter((p) => p.thscode && p.shares);
  const bad = rows.filter((r) => !(Number(r.shares) > 0) || (r.cost !== "" && !(Number(r.cost) > 0)));
  const submit = () =>
    save.mutate({
      accountId: id,
      data: {
        cash: Number(cash) || 0,
        positions: rows.map((r) => ({
          thscode: r.thscode,
          shares: Math.round(Number(r.shares)),
          cost_price: r.cost === "" ? null : Number(r.cost),
          opened_on: r.opened_on || null,
        })),
      },
    });
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-end gap-3">
        <Field label="现金（元）">
          <input
            className={`${inputClass} w-40`}
            inputMode="decimal"
            value={cash}
            onChange={(e) => setCash(e.target.value)}
          />
        </Field>
        <SecuritySearch label="加一只股票" onPick={(c) => add(c)} />
      </div>

      <details className="rounded-lg border border-dashed border-zinc-300 p-3 text-sm dark:border-zinc-700">
        <summary className="cursor-pointer text-zinc-600 dark:text-zinc-400">从券商 App 复制持仓，粘贴到这里</summary>
        <p className="mt-2 text-xs text-zinc-500">
          每行一只股票，有 6
          位代码或简称、股数就能认出来；带小数点的数当作成本价。表头、合计等行会跳过；认不出的股票标红列在下面，不会加进去。
        </p>
        <textarea
          className={`${inputClass} mt-2 h-28 w-full font-mono text-xs`}
          placeholder={"600519 贵州茅台 100 100 1498.000\n000858 五粮液 200 200 142.50"}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <div className="mt-2 flex items-center gap-2">
          <button
            type="button"
            className={buttonClass("ghost")}
            disabled={!text.trim() || parse.isPending}
            onClick={() => parse.mutate({ accountId: id, data: { text } })}
          >
            {parse.isPending ? "识别中…" : "识别"}
          </button>
          {parsed && (
            <span className="text-xs text-zinc-500">
              认出 {usable.length} 只
              {parsed.length > usable.length && `，${parsed.length - usable.length} 行没认全（标红的不会加）`}
            </span>
          )}
          {usable.length > 0 && (
            <button
              type="button"
              className={buttonClass()}
              onClick={() => {
                for (const p of usable)
                  add(p.thscode ?? "", p.name ?? "", String(p.shares), p.cost_price ? String(p.cost_price) : "");
                setParsed(null);
                setText("");
              }}
            >
              加到下面
            </button>
          )}
        </div>
        {parsed && parsed.length > 0 && (
          <ul className="mt-2 space-y-0.5 text-xs">
            {parsed.map((p) => (
              <li key={p.line} className={p.problem ? "text-red-600" : "text-zinc-600 dark:text-zinc-400"}>
                {p.name ?? "?"} {p.thscode} · {p.shares ?? "?"} 股{p.cost_price ? ` · 成本 ${p.cost_price}` : ""}
                {p.problem && `（${p.problem}）`}
                <span className="ml-2 text-zinc-400">{p.line}</span>
              </li>
            ))}
          </ul>
        )}
        {!!parse.error && <p className="mt-2 text-sm text-red-600">{errorText(parse.error)}</p>}
      </details>

      {rows.length === 0 ? (
        <p className="text-sm text-zinc-500">还没有持仓。空仓也可以直接保存，系统会给出买入建议。</p>
      ) : (
        <div className="-mx-4 overflow-x-auto">
          <table className="w-full min-w-max text-sm [&_td]:px-2 [&_td]:py-1.5 [&_th]:px-2 [&_th]:py-1 [&_th]:text-left [&_th]:text-xs [&_th]:font-medium [&_th]:text-zinc-500">
            <thead>
              <tr>
                <th className="pl-4!">股票</th>
                <th>股数</th>
                <th>成本价（可不填）</th>
                <th>买入日（可不填）</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {rows.map((r, i) => (
                <tr key={r.thscode} className="border-t border-zinc-100 dark:border-zinc-800">
                  <td className="pl-4!">
                    <NameOf code={r.thscode} fallback={r.name} />
                    <span className="ml-1 text-xs text-zinc-400">{r.thscode}</span>
                  </td>
                  <td>
                    <input
                      className={`${inputClass} w-24`}
                      inputMode="numeric"
                      value={r.shares}
                      onChange={(e) => set(i, { shares: e.target.value })}
                    />
                  </td>
                  <td>
                    <input
                      className={`${inputClass} w-28`}
                      inputMode="decimal"
                      value={r.cost}
                      onChange={(e) => set(i, { cost: e.target.value })}
                    />
                  </td>
                  <td>
                    <input
                      className={`${inputClass} w-36`}
                      type="date"
                      value={r.opened_on}
                      onChange={(e) => set(i, { opened_on: e.target.value })}
                    />
                  </td>
                  <td>
                    <button
                      type="button"
                      className="text-xs text-zinc-500 hover:text-red-600"
                      onClick={() => setRows((x) => x.filter((_, k) => k !== i))}
                    >
                      去掉
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      <p className="text-xs text-zinc-500">
        填上成本价才能按策略止损、止盈判断。分红送转之后系统会自动改股数和成本，不用自己改；这里没列出的股票视为已经卖掉。
      </p>
      {bad.length > 0 && <p className="text-sm text-red-600">股数要大于 0，成本价不填或大于 0</p>}
      {!!save.error && <p className="text-sm text-red-600">{errorText(save.error)}</p>}
      <div>
        <button type="button" className={buttonClass()} disabled={save.isPending || bad.length > 0} onClick={submit}>
          {save.isPending ? "保存中…" : saveLabel}
        </button>
      </div>
    </div>
  );
}
