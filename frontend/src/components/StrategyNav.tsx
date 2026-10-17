import { clsx } from "clsx";
import { NavLink } from "react-router";

const TABS = [
  { to: "/strategies", label: "我的策略", hint: "买入条件 + 卖出规则，每天收盘后给出买点和卖点" },
  { to: "/signals", label: "信号", hint: "起了名字的条件，例如「均线金叉」「放量」；可以自己定义" },
  { to: "/indicators", label: "指标", hint: "每天算出的一个数，例如均线、RSI；信号和策略都由它们组成" },
];

/** 策略栏目下的三页：策略由信号和指标组成，三者互相链接 */
export function StrategyNav() {
  return (
    <nav className="flex gap-1 border-b border-zinc-200 dark:border-zinc-800">
      {TABS.map((t) => (
        <NavLink
          key={t.to}
          to={t.to}
          title={t.hint}
          className={({ isActive }) =>
            clsx(
              "-mb-px border-b-2 px-3 py-2 text-sm whitespace-nowrap transition",
              isActive
                ? "border-accent font-medium text-zinc-900 dark:text-zinc-100"
                : "border-transparent text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100",
            )
          }
        >
          {t.label}
        </NavLink>
      ))}
    </nav>
  );
}
