import type { ReactNode } from "react";
import { Link } from "react-router";
import { Card, PageHeader } from "../components/ui";

/** 说明页：指标、条件、信号、策略四个概念和买卖点的规则。每个信号、指标的详细说明在各自的页面上 */
export function GuidePage() {
  return (
    <>
      <PageHeader title="说明" desc="牛来里的选股方法由四层东西搭起来，下一层由上一层组成，每一层都能点进去看清楚。" />
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <Concept n="1" title="指标" to="/indicators" link="看全部指标" example="例：20 日均线、RSI、成交额">
          每个交易日算出的一个数，描述股票当天的某个方面。
        </Concept>
        <Concept n="2" title="条件" example="例：收盘价上穿 60 日均线">
          指标和一个数或另一个指标比较：高于、不低于、低于、不高于、上穿、下穿；再加上时间范围：当天、最近几天内出现过、连续几天。
        </Concept>
        <Concept n="3" title="信号" to="/signals" link="看全部信号" example="例：均线交叉、放量、创新高">
          起了名字的条件，一条或几条组合。<span className="text-up">红色偏多</span>，常被看作走强；
          <span className="text-down">绿色偏空</span>，常被看作走弱；
          <span className="text-amber-600 dark:text-amber-400">橙色中性</span>，本身不说明方向。可以自己定义。
        </Concept>
        <Concept n="4" title="策略" to="/strategies" link="看策略" example="例：均线交叉时买，跌破 20 日线时卖">
          买入条件 + 卖出规则。每天收盘后在全市场算一遍，给出最新买点、最新卖点和持有中。
        </Concept>
      </div>
      <Card title="买点和卖点怎么算">
        <ul className="list-disc space-y-1 pl-5 text-sm">
          <li>收盘后才知道当天是否满足条件，所以买卖都按「下一个交易日开盘」算。</li>
          <li>空仓时满足买入条件（同一天没有满足卖出条件），就是买点；已经持有时再满足，不算新的买点。</li>
          <li>
            持有中满足任意一条卖出规则（卖出信号、止损、止盈、最多持有天数），就是卖点。同一天几条都满足时，按卖出信号、止损、止盈、持有到期的顺序写原因。
          </li>
          <li>开盘一字涨停买不进、一字跌停卖不出时，顺延到下一个交易日。</li>
          <li>「持有中」是按这些规则从一年前开始推算出来的，不是你的实际持仓。回测用的也是同一套规则。</li>
        </ul>
        <p className="mt-3 text-xs text-zinc-500">所有信号和策略都只是对过去数据的整理，不构成投资建议。</p>
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
