import { keepPreviousData } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link } from "react-router";
import { useListSecurities } from "../api/generated/market/market";
import { Badge, buttonClass, Card, Empty, inputClass, PageHeader, QueryView, Segmented, Table } from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { useDebounced } from "../hooks/useDebounced";
import { useSearchState } from "../hooks/useSearchState";
import { fmtInt } from "../lib/format";
import { BOARDS } from "../lib/labels";

const PAGE = 50;
const BOARD_OPTIONS = [
  { value: "", label: "全部" },
  ...Object.entries(BOARDS).map(([value, label]) => ({ value, label })),
];

export function SecuritiesPage() {
  const [s, set] = useSearchState({ q: "", board: "", page: "1" });
  const [text, setText] = useState(s.q);
  const q = useDebounced(text);
  useEffect(() => {
    if (q !== s.q) set({ q, page: "1" });
  }, [q, s.q, set]);
  const page = Math.max(1, Number(s.page) || 1);
  const query = useListSecurities(
    { q: s.q || undefined, board: s.board || undefined, limit: PAGE, offset: (page - 1) * PAGE },
    { query: { placeholderData: keepPreviousData } },
  );

  return (
    <>
      <PageHeader title="股票" desc="沪深京 A 股在市股票。点击股票查看 K 线、复权价格和均线信号。" />
      <Card>
        <div className="mb-4 flex flex-wrap items-center gap-3">
          <input
            className={`${inputClass} w-64`}
            placeholder="代码或名称，例如 600519 或 茅台"
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
          <Segmented value={s.board} options={BOARD_OPTIONS} onChange={(board) => set({ board, page: "1" })} />
        </div>
        <QueryView query={query}>
          {(data) =>
            data.items.length === 0 ? (
              <Empty>没有符合条件的股票</Empty>
            ) : (
              <>
                <Table>
                  <thead>
                    <tr>
                      <th className="w-0" aria-label="收藏" />
                      <th>代码</th>
                      <th>名称</th>
                      <th>板块</th>
                      <th>交易所</th>
                      <th>上市日期</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.items.map((x) => (
                      <tr key={x.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
                        <td className="pr-0!">
                          <WatchButton thscode={x.thscode} />
                        </td>
                        <td className="num font-mono">
                          <Link to={`/securities/${x.thscode}`} className="text-accent hover:underline">
                            {x.thscode}
                          </Link>
                        </td>
                        <td>
                          <Link to={`/securities/${x.thscode}`} className="hover:underline">
                            {x.name}
                          </Link>
                          {x.name.includes("ST") && (
                            <span className="ml-2">
                              <Badge tone="warn">ST</Badge>
                            </span>
                          )}
                        </td>
                        <td>{BOARDS[x.board] ?? x.board}</td>
                        <td>{x.exchange}</td>
                        <td className="num">{x.list_date ?? "未上市"}</td>
                      </tr>
                    ))}
                  </tbody>
                </Table>
                <Pager page={page} total={data.total} onChange={(p) => set({ page: String(p) })} />
              </>
            )
          }
        </QueryView>
      </Card>
    </>
  );
}

export function Pager({ page, total, onChange }: { page: number; total: number; onChange: (p: number) => void }) {
  const pages = Math.max(1, Math.ceil(total / PAGE));
  return (
    <div className="mt-4 flex items-center justify-between text-sm text-zinc-500">
      <span>共 {fmtInt(total)} 只</span>
      <div className="flex items-center gap-2">
        <button type="button" className={buttonClass("ghost")} disabled={page <= 1} onClick={() => onChange(page - 1)}>
          上一页
        </button>
        <span className="num">
          {page} / {pages}
        </span>
        <button
          type="button"
          className={buttonClass("ghost")}
          disabled={page >= pages}
          onClick={() => onChange(page + 1)}
        >
          下一页
        </button>
      </div>
    </div>
  );
}
