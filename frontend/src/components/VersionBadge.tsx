import { useQueryClient } from "@tanstack/react-query";
import { clsx } from "clsx";
import { useEffect, useRef, useState } from "react";
import type { VersionInfo } from "../api/generated/model";
import { getVersionQueryKey, useUpgrade, useVersion, version } from "../api/generated/system/system";
import { buttonClass, errorText } from "./ui";

/** 页头 logo 后面的版本号：有新版本时亮红点，点开可以一键升级（Docker 部署启用了 upgrader 时）。
 * 升级时服务会重启，期间请求失败是正常的；看到运行的版本变成目标版本后刷新页面 */
export function VersionBadge() {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const [waiting, setWaiting] = useState<string | null>(null);
  const q = useVersion(undefined, {
    query: {
      staleTime: 60_000,
      refetchInterval: (query) => {
        const s = query.state.data?.upgrade.state;
        return waiting || s === "requested" || s === "running" ? 2000 : 30 * 60_000;
      },
      // 服务重启时请求会失败，继续轮询
      retry: waiting ? true : 1,
    },
  });
  const upgrade = useUpgrade({
    mutation: {
      onSuccess: (_, vars) => {
        setWaiting(vars.data.version ?? null);
        qc.invalidateQueries({ queryKey: getVersionQueryKey() });
      },
    },
  });
  const v = q.data;

  // 新版本已经在运行：刷新页面，加载新的前端
  useEffect(() => {
    if (waiting && v?.current === waiting) {
      const t = setTimeout(() => window.location.reload(), 1500);
      return () => clearTimeout(t);
    }
  }, [waiting, v?.current]);
  // 升级失败时停止等待
  useEffect(() => {
    if (waiting && v?.upgrade.state === "failed" && v.upgrade.target === waiting) setWaiting(null);
  }, [waiting, v?.upgrade.state, v?.upgrade.target]);

  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => ref.current && !ref.current.contains(e.target as Node) && setOpen(false);
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [open]);

  if (!v) return null;
  const busy = !!waiting || v.upgrade.state === "requested" || v.upgrade.state === "running";
  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        title={v.has_update ? `有新版本 v${v.latest}` : "当前版本"}
        className="flex items-center gap-1.5 rounded-md px-1.5 py-0.5 text-xs text-zinc-500 hover:bg-zinc-100 dark:hover:bg-zinc-800"
      >
        <span className="num">{v.current === "dev" ? "开发版" : `v${v.current}`}</span>
        {busy ? (
          <span className="size-2 animate-pulse rounded-full bg-accent" />
        ) : (
          v.has_update && <span className="size-2 rounded-full bg-up" />
        )}
        {v.has_update && !busy && <span className="hidden text-up sm:inline">新版本</span>}
      </button>
      {open && (
        <div className="absolute left-0 z-30 mt-2 w-80 rounded-xl border border-zinc-200 bg-white p-4 text-sm shadow-lg dark:border-zinc-700 dark:bg-zinc-900">
          <Panel
            v={v}
            busy={busy}
            waiting={waiting}
            upgradeError={upgrade.error}
            upgrading={upgrade.isPending}
            onUpgrade={() => v.latest && upgrade.mutate({ data: { version: v.latest } })}
            onRefresh={async () => qc.setQueryData(getVersionQueryKey(), await version({ refresh: true }))}
          />
        </div>
      )}
    </div>
  );
}

function Panel({
  v,
  busy,
  waiting,
  upgradeError,
  upgrading,
  onUpgrade,
  onRefresh,
}: {
  v: VersionInfo;
  busy: boolean;
  waiting: string | null;
  upgradeError: unknown;
  upgrading: boolean;
  onUpgrade: () => void;
  onRefresh: () => void;
}) {
  const u = v.upgrade;
  if (busy) {
    const done = waiting !== null && v.current === waiting;
    return (
      <div className="flex flex-col gap-2">
        <p className="flex items-center gap-2 font-medium">
          <span className="size-3.5 animate-spin rounded-full border-2 border-zinc-300 border-t-accent" />
          {done ? `已升级到 v${waiting}，正在刷新页面` : `正在升级到 v${u.target ?? waiting}`}
        </p>
        {!done && <p className="text-zinc-500">{u.message ?? "等待升级助手接手"}</p>}
        <p className="text-xs text-zinc-400">
          升级时服务会重启一次，约一到两分钟，期间页面可能暂时打不开。完成后自动刷新。
        </p>
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-3">
      <dl className="num grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
        <dt className="text-zinc-500">当前版本</dt>
        <dd>{v.current === "dev" ? "开发版（本机编译）" : `v${v.current}`}</dd>
        <dt className="text-zinc-500">最新版本</dt>
        <dd className="flex items-center gap-2">
          {v.latest ? `v${v.latest}` : "—"}
          <button type="button" onClick={onRefresh} className="text-xs text-accent hover:underline">
            重新检查
          </button>
        </dd>
      </dl>
      {v.check_error && <p className="text-xs text-amber-600">{v.check_error}</p>}

      {u.state === "failed" && (
        <div className="rounded-lg bg-red-50 p-2 text-xs text-red-700 dark:bg-red-500/10 dark:text-red-300">
          <p>
            上次升级{u.target ? `到 v${u.target}` : ""}失败：{u.message}
          </p>
          {u.log_tail && (
            <details className="mt-1">
              <summary className="cursor-pointer select-none">日志</summary>
              <pre className="mt-1 max-h-40 overflow-auto text-[11px] whitespace-pre-wrap">{u.log_tail}</pre>
            </details>
          )}
        </div>
      )}

      {!v.has_update ? (
        <p className="text-zinc-500">{v.current === "dev" ? "开发版不检查更新。" : "已是最新版本。"}</p>
      ) : (
        <>
          {v.changes_url && (
            <a href={v.changes_url} target="_blank" rel="noreferrer" className="text-accent hover:underline">
              看看 v{v.latest} 改了什么 →
            </a>
          )}
          {u.available ? (
            <>
              <button type="button" className={clsx(buttonClass(), "w-full")} disabled={upgrading} onClick={onUpgrade}>
                {upgrading ? "提交中…" : `一键升级到 v${v.latest}`}
              </button>
              <p className="text-xs text-zinc-400">
                会先备份数据库，再下载新版本并重启服务，约一到两分钟。新版本启动失败时自动换回当前版本。
              </p>
            </>
          ) : (
            <div className="text-xs text-zinc-500">
              <p>
                这个部署没有启用一键升级。手动升级：把 .env 中的 NIULAI_IMAGE 改成新版本号，在 compose.yml 所在目录执行
              </p>
              <pre className="mt-1 rounded bg-zinc-100 p-2 text-[11px] dark:bg-zinc-800">
                docker compose pull{"\n"}docker compose up -d
              </pre>
              <p className="mt-1">启用一键升级的方法见运维手册的「一键升级」。</p>
            </div>
          )}
          {!!upgradeError && <p className="text-xs text-red-600">{errorText(upgradeError)}</p>}
        </>
      )}
    </div>
  );
}
