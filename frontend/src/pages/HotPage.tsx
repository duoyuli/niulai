import { Link } from "react-router";
import { useMarketHot } from "../api/generated/market/market";
import type { HotLimitUpGroup, HotLimitUpStock, HotMarket, HotStock } from "../api/generated/model";
import { Badge, Card, Empty, inputClass, PageHeader, QueryView, Table } from "../components/ui";
import { useSearchState } from "../hooks/useSearchState";
import { dayLabel, fmtBig, fmtInt, fmtPctValue, fmtPrice, todayCn, upDownClass } from "../lib/format";

/** 今日热点：权威涨停、跌停、炸板池，按连板高度分组。数据每周同步，可以切换日期往回看 */
export function HotPage() {
  const [s, set] = useSearchState({ date: "" });
  const q = useMarketHot({ date: s.date || undefined });
  return (
    <>
      <PageHeader
        title="今日热点"
        desc="涨停、跌停、炸板（盘中涨停又打开）的股票，来自交易所口径的权威数据。点股票名可以看它最近的走势。"
      />
      <Card>
        <div className="flex flex-wrap items-center gap-3">
          <label className="flex items-center gap-2 text-sm text-zinc-500">
            看哪一天
            <input
              type="date"
              className={`${inputClass} w-40`}
              value={s.date}
              max={todayCn()}
              onChange={(e) => set({ date: e.target.value })}
            />
          </label>
          <span className="text-xs text-zinc-500">
            不选就是本地最新的一天。涨跌停数据每周同步一次，最近几天可能还没更新。
          </span>
        </div>
      </Card>
      <QueryView query={q}>{(d) => <HotDetail d={d} />}</QueryView>
    </>
  );
}

function HotDetail({ d }: { d: HotMarket }) {
  if (!d.available) {
    return (
      <Card>
        <Empty>{d.note ?? "这一天的涨停数据还没同步，稍后再看"}</Empty>
      </Card>
    );
  }
  const stats = d.stats;
  return (
    <>
      <Card>
        <div className="flex flex-wrap items-baseline gap-x-8 gap-y-3">
          <Summary label="涨停" value={`${fmtInt(d.limit_up_total)} 家`} className="text-up" />
          <Summary label="跌停" value={`${fmtInt(d.limit_down_total)} 家`} className="text-down" />
          <Summary label="炸板" value={`${fmtInt(d.broken_total)} 家`} />
          <Summary
            label="最高连板"
            value={d.max_board >= 2 ? `${d.max_board} 板` : d.limit_up_total > 0 ? "首板" : "—"}
          />
          {stats?.seal_rate != null && (
            <Summary
              label="封板率"
              value={`${(stats.seal_rate * 100).toFixed(0)}%`}
              hint="收盘还封在涨停的，占盘中触及过涨停的比例"
            />
          )}
          {d.trade_date && <span className="text-xs text-zinc-500">交易日 {dayLabel(d.trade_date)}</span>}
        </div>
        {stats && (
          <p className="mt-3 text-xs text-zinc-500">
            盘中触及过涨停 {fmtInt(stats.touched_limit_up_count)} 家，其中 {fmtInt(stats.limit_up_count)} 家收盘封住、
            {fmtInt(stats.open_count)} 家在盘中打开。
          </p>
        )}
        <p className="mt-2 text-xs text-zinc-500">
          「涨停」是涨到当天允许的上限；「炸板」是盘中涨停过、收盘又没封住；「跌停」是跌到当天的下限。
        </p>
      </Card>
      {d.limit_up_groups.length === 0 ? (
        <Card>
          <Empty>这一天没有涨停的股票。</Empty>
        </Card>
      ) : (
        d.limit_up_groups.map((g) => <GroupCard key={g.board_num} g={g} />)
      )}
      <Card
        title={
          <>
            跌停
            <span className="ml-2 text-xs font-normal text-zinc-500">
              共 {fmtInt(d.limit_down_total)} 家
              {d.limit_down_total > d.limit_down_stocks.length ? `，列出前 ${d.limit_down_stocks.length} 只` : ""}
            </span>
          </>
        }
      >
        {d.limit_down_stocks.length === 0 ? (
          <Empty>这一天没有跌停的股票。</Empty>
        ) : (
          <DownTable rows={d.limit_down_stocks} />
        )}
      </Card>
      <Card
        title={
          <>
            炸板
            <span className="ml-2 text-xs font-normal text-zinc-500">
              共 {fmtInt(d.broken_total)} 家
              {d.broken_total > d.broken_stocks.length ? `，列出前 ${d.broken_stocks.length} 只` : ""}
            </span>
          </>
        }
      >
        {d.broken_stocks.length === 0 ? <Empty>这一天没有炸板的股票。</Empty> : <BrokenTable rows={d.broken_stocks} />}
      </Card>
      <p className="text-xs text-zinc-500">
        数据由同花顺整理、每周同步一次；涨停池从 2020 年 7 月开始有，跌停和炸板只有 2025 年 10
        月以后的数据，更早的日期只显示涨停。
      </p>
    </>
  );
}

function Summary({
  label,
  value,
  className,
  hint,
}: {
  label: string;
  value: string;
  className?: string;
  hint?: string;
}) {
  return (
    <div className="flex flex-col gap-0.5" title={hint}>
      <span className="text-xs text-zinc-500">{label}</span>
      <span className={`num text-lg font-semibold ${className ?? ""}`}>{value}</span>
    </div>
  );
}

function GroupCard({ g }: { g: HotLimitUpGroup }) {
  return (
    <Card
      title={
        <>
          {g.label}
          <span className="ml-2 text-xs font-normal text-zinc-500">
            {fmtInt(g.count)} 家{g.count > g.stocks.length ? `，列出前 ${g.stocks.length} 只` : ""}
          </span>
        </>
      }
    >
      {g.stocks.length === 0 ? (
        <Empty>这一档没有股票。</Empty>
      ) : (
        <Table>
          <thead>
            <tr>
              <th>名称</th>
              <th>代码</th>
              <th>连板</th>
              <th>涨停原因</th>
              <th className="text-right!">最新价</th>
              <th className="text-right!" title="收盘时挂在这个价位上想买、没买到的钱">
                封单额
              </th>
              <th>首次涨停</th>
            </tr>
          </thead>
          <tbody>
            {g.stocks.map((x) => (
              <LimitUpRow key={x.thscode} x={x} />
            ))}
          </tbody>
        </Table>
      )}
    </Card>
  );
}

function LimitUpRow({ x }: { x: HotLimitUpStock }) {
  return (
    <tr className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
      <td>
        <StockLink thscode={x.thscode} name={x.name} />
        {x.is_st && (
          <span className="ml-1.5">
            <Badge tone="muted">ST</Badge>
          </span>
        )}
      </td>
      <td className="num font-mono text-xs text-zinc-500">{x.thscode}</td>
      <td>{x.continue_day_text ?? "—"}</td>
      <td className="max-w-64 truncate" title={x.limit_up_reason ?? undefined}>
        {x.limit_up_reason ?? "—"}
      </td>
      <td className="num text-right">{fmtPrice(x.last_price)}</td>
      <td className="num text-right">{fmtBig(x.seal_money)}</td>
      <td className="num">{x.limit_up_time ?? "—"}</td>
    </tr>
  );
}

function DownTable({ rows }: { rows: HotStock[] }) {
  return (
    <Table>
      <thead>
        <tr>
          <th>名称</th>
          <th>代码</th>
          <th className="text-right!">最新价</th>
          <th className="text-right!">涨跌幅</th>
          <th>首次跌停</th>
          <th>最后一次跌停</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((x) => (
          <tr key={x.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
            <td>
              <StockLink thscode={x.thscode} name={x.name} />
            </td>
            <td className="num font-mono text-xs text-zinc-500">{x.thscode}</td>
            <td className="num text-right">{fmtPrice(x.last_price)}</td>
            <td className={`num text-right ${upDownClass(x.price_change_pct)}`}>
              {fmtPctValue(x.price_change_pct, 1)}
            </td>
            <td className="num">{x.first_limit_time ?? "—"}</td>
            <td className="num">{x.last_limit_time ?? "—"}</td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}

function BrokenTable({ rows }: { rows: HotStock[] }) {
  return (
    <Table>
      <thead>
        <tr>
          <th>名称</th>
          <th>代码</th>
          <th className="text-right!">最新价</th>
          <th className="text-right!">涨跌幅</th>
          <th className="text-right!" title="盘中打开涨停的次数">
            炸板次数
          </th>
          <th className="text-right!">成交额</th>
          <th className="text-right!">换手率</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((x) => (
          <tr key={x.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
            <td>
              <StockLink thscode={x.thscode} name={x.name} />
            </td>
            <td className="num font-mono text-xs text-zinc-500">{x.thscode}</td>
            <td className="num text-right">{fmtPrice(x.last_price)}</td>
            <td className={`num text-right ${upDownClass(x.price_change_pct)}`}>
              {fmtPctValue(x.price_change_pct, 1)}
            </td>
            <td className="num text-right">{fmtInt(x.open_times)}</td>
            <td className="num text-right">{fmtBig(x.turnover)}</td>
            <td className="num text-right">
              {x.turnover_ratio_pct == null ? "—" : `${x.turnover_ratio_pct.toFixed(2)}%`}
            </td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}

function StockLink({ thscode, name }: { thscode: string; name: string }) {
  return (
    <Link to={`/securities/${thscode}`} className="font-medium hover:text-accent hover:underline">
      {name || thscode}
    </Link>
  );
}
