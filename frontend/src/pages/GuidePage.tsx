import type { ReactNode } from "react";
import { Link } from "react-router";
import { Card, PageHeader } from "../components/ui";

/** 说明页：牛来怎么选股（因子、排名、组合、账户调仓四步）和建议怎么执行。规则见 docs/domains/accounts.md */
export function GuidePage() {
  return (
    <>
      <PageHeader
        title="说明"
        desc="牛来只用一种选股方法：每月给全市场股票按几个特征打分，分散持有排名靠前的一批。下面四步说明它是怎么来的。"
      />
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <Concept n="1" title="因子" to="/research" link="看因子研究" example="例：波动率、特质波动率、单季利润增速">
          描述一只股票某个方面的一个数。每个因子有预期的方向，例如 A 股里波动小的股票之后往往表现更好。
        </Concept>
        <Concept n="2" title="排名" example="例：同一天 3,700 只股票从 1 排到 3,700">
          每个交易日收盘，把几个因子去极值、按行业和规模中性化后合成一个分数，给全市场股票排名。排除 ST、股价低于 2
          元、成交额不到 5,000 万元和上市不满 60 个交易日的股票。
        </Concept>
        <Concept n="3" title="组合" example="例：持有前 30 只，每只约 1/30">
          持有排名最靠前的一批，等额买入。已持有的股票只要还在缓冲区以内就继续拿，减少来回买卖的成本。
        </Concept>
        <Concept n="4" title="账户调仓" to="/accounts" link="看我的账户" example="例：卖出 3 只、买入 3 只">
          每月最后一个交易日收盘后，按新的排名给出下一个交易日的买卖清单；没有持满时每天补仓。
        </Concept>
      </div>
      <Card title="建议怎么执行">
        <ul className="list-disc space-y-1 pl-5 text-sm">
          <li>收盘后才知道排名，所以买卖都按「下一个交易日开盘」执行。</li>
          <li>买入条写了最高买入价：开盘价高于它就是追高，这只放弃；开盘一字涨停买不进，这条作废，之后会继续补。</li>
          <li>卖出开盘后执行；一字跌停卖不出时，下一份建议会继续提示。</li>
          <li>持有的股票被 ST 时卖出；停牌、退市的先不动。</li>
          <li>照着做完在账户里点「已执行」，持仓和现金自动更新；分红送转按公告自动记账。</li>
        </ul>
        <p className="mt-3 text-xs text-zinc-500">
          排名只是对过去数据的整理，纯做多的组合会跟着大盘涨跌，不构成投资建议。
        </p>
      </Card>
    </>
  );
}

function Concept({
  n,
  title,
  example,
  to,
  link,
  children,
}: {
  n: string;
  title: string;
  example: string;
  to?: string;
  link?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-col rounded-xl border border-zinc-200 bg-white p-4 dark:border-zinc-800 dark:bg-zinc-900">
      <h2 className="flex items-center gap-2 font-semibold">
        <span className="num flex size-5 items-center justify-center rounded-full bg-accent/10 text-xs text-accent">
          {n}
        </span>
        {title}
      </h2>
      <p className="mt-1 text-sm text-zinc-600 dark:text-zinc-400">{children}</p>
      <p className="mt-2 text-xs text-zinc-400">{example}</p>
      {to && (
        <Link to={to} className="mt-auto pt-2 text-sm text-accent hover:underline">
          {link} →
        </Link>
      )}
    </div>
  );
}
