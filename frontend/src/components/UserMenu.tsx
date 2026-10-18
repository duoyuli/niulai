import { clsx } from "clsx";
import { useEffect, useRef, useState } from "react";
import { Link } from "react-router";
import { useLogout } from "../api/generated/auth/auth";
import type { AuthStatus } from "../api/generated/model";

/** 页头右侧的用户名：鼠标移上去或点击展开菜单。不常用的功能（改密码、用户管理、数据源设置、退出）都收在这里 */
export function UserMenu({ me }: { me: AuthStatus }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const logout = useLogout({ mutation: { onSuccess: () => window.location.assign("/login") } });
  const admin = me.role === "admin";
  // 点菜单外面或按 Esc 收起
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent ? e.key === "Escape" : !box.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", close);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", close);
    };
  }, [open]);
  const items = [
    { to: "/me", label: "修改密码" },
    ...(admin
      ? [
          { to: "/users", label: "用户管理" },
          { to: "/setup", label: "数据源设置" },
        ]
      : []),
  ];
  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: 鼠标悬停只是方便展开，点击按钮同样可以
    <div ref={box} className="relative" onMouseEnter={() => setOpen(true)} onMouseLeave={() => setOpen(false)}>
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className={clsx(
          "flex items-center gap-1.5 rounded-full py-0.5 pr-2 pl-0.5 text-xs whitespace-nowrap transition",
          open
            ? "bg-zinc-100 text-zinc-900 dark:bg-zinc-800 dark:text-zinc-100"
            : "text-zinc-600 hover:bg-zinc-100 hover:text-zinc-900 dark:text-zinc-400 dark:hover:bg-zinc-800 dark:hover:text-zinc-100",
        )}
      >
        <span className="flex size-5 items-center justify-center rounded-full bg-accent/15 text-[11px] font-semibold text-accent uppercase">
          {me.username.slice(0, 1)}
        </span>
        <span className="hidden max-w-24 truncate sm:inline">{me.username}</span>
        <span className="text-[10px] text-zinc-400">▾</span>
      </button>
      {open && (
        // 外层留出上边距当作「桥」，鼠标从名字移到菜单时不会收起
        <div className="absolute right-0 top-full z-20 pt-1.5">
          <div
            role="menu"
            className="w-44 overflow-hidden rounded-lg border border-zinc-200 bg-white py-1 text-sm shadow-lg dark:border-zinc-700 dark:bg-zinc-900"
          >
            <div className="border-b border-zinc-100 px-3 py-2 dark:border-zinc-800">
              <p className="truncate font-medium">{me.username}</p>
              <p className="text-xs text-zinc-500">{admin ? "管理员" : "普通用户"}</p>
            </div>
            {items.map((i) => (
              <Link
                key={i.to}
                to={i.to}
                role="menuitem"
                onClick={() => setOpen(false)}
                className="block px-3 py-1.5 text-zinc-700 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800"
              >
                {i.label}
              </Link>
            ))}
            <div className="my-1 border-t border-zinc-100 dark:border-zinc-800" />
            <button
              type="button"
              role="menuitem"
              disabled={logout.isPending}
              onClick={() => logout.mutate()}
              className="block w-full px-3 py-1.5 text-left text-zinc-700 hover:bg-zinc-100 dark:text-zinc-300 dark:hover:bg-zinc-800"
            >
              退出登录
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
