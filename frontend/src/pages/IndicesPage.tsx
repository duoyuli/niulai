import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router";
import { useListIndices } from "../api/generated/market/market";
import { Card, Empty, inputClass, PageHeader, QueryView, Segmented, Table } from "../components/ui";
import { useDebounced } from "../hooks/useDebounced";
import { useSearchState } from "../hooks/useSearchState";
import { fmtInt } from "../lib/format";
import { INDEX_CATEGORIES } from "../lib/labels";

const CATEGORY_OPTIONS = [
  { value: "", label: "全部" },
  ...["exchange", "industry", "cn_concept", "region", "tszs", "ths"].map((value) => ({
    value,
    label: INDEX_CATEGORIES[value] ?? value,
  })),
];

export function IndicesPage() {
  const [s, set] = useSearchState({ category: "", q: "" });
  const [text, setText] = useState(s.q);
  const q = useDebounced(text);
  useEffect(() => {
    if (q !== s.q) set({ q });
  }, [q, s.q, set]);
  const query = useListIndices({ category: s.category || undefined, q: s.q || undefined });

  return (
    <>
      <PageHeader
        title="指数与板块"
        desc="交易所指数（如沪深 300）和同花顺的行业、概念、地域板块。点击查看走势和成分股。"
      />
      <Card>
        <div className="mb-4 flex flex-wrap items-center gap-3">
          <input
            className={`${inputClass} w-56`}
            placeholder="代码或名称，例如 沪深300"
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
          <Segmented value={s.category} options={CATEGORY_OPTIONS} onChange={(category) => set({ category })} />
        </div>
        <QueryView query={query}>{(data) => <IndexTable rows={data} />}</QueryView>
      </Card>
    </>
  );
}

function IndexTable({ rows }: { rows: { thscode: string; name: string; category: string; members: number }[] }) {
  const [limit, setLimit] = useState(200);
  const shown = useMemo(() => rows.slice(0, limit), [rows, limit]);
  if (rows.length === 0) return <Empty>没有符合条件的指数</Empty>;
  return (
    <>
      <Table>
        <thead>
          <tr>
            <th>代码</th>
            <th>名称</th>
            <th>分类</th>
            <th className="text-right!" title="当前成分股数量；0 表示还没有采集到成分">
              成分股
            </th>
          </tr>
        </thead>
        <tbody>
          {shown.map((x) => (
            <tr key={x.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
              <td className="num font-mono">
                <Link to={`/indices/${x.thscode}`} className="text-accent hover:underline">
                  {x.thscode}
                </Link>
              </td>
              <td>{x.name}</td>
              <td>{INDEX_CATEGORIES[x.category] ?? x.category}</td>
              <td className="num text-right">{x.members > 0 ? fmtInt(x.members) : "—"}</td>
            </tr>
          ))}
        </tbody>
      </Table>
      <div className="mt-4 flex items-center justify-between text-sm text-zinc-500">
        <span>共 {fmtInt(rows.length)} 个</span>
        {rows.length > limit && (
          <button type="button" className="text-accent hover:underline" onClick={() => setLimit((l) => l + 200)}>
            显示更多
          </button>
        )}
      </div>
    </>
  );
}
