import { useQueryClient } from "@tanstack/react-query";
import { clsx } from "clsx";
import { useEffect, useRef, useState } from "react";
import type { BackupStatus, UpgradeStatus, UpgradeStep, VersionInfo } from "../api/generated/model";
import { getVersionQueryKey, useUpgrade, useVersion, version } from "../api/generated/system/system";
import { fmtAgo } from "../lib/format";
import { buttonClass, errorText } from "./ui";

/** 页头 logo 后面的版本号：有新版本时亮红点，点开可以一键升级（Docker 部署启用了 upgrader 时）。
 * 升级进度只在这个弹窗里显示，收起后版本号上的圆点一直闪，再点开接着看。升级时服务会重启，期间请求失败是正常的；看到运行的版本变成目标版本后刷新页面 */
export function VersionBadge() {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const [waiting, setWaiting] = useState<string | null>(null);
  // 升级进行中（包括升级助手启动后做部署迁移）：服务会重启，请求失败也继续轮询
  const busyRef = useRef(false);
  const q = useVersion(undefined, {
    query: {
      staleTime: 60_000,
      refetchInterval: (query) => {
        const s = query.state.data?.upgrade.state;
        return waiting || s === "requested" || s === "running" ? 2000 : 30 * 60_000;
      },
      // 服务重启时请求会失败，继续轮询
      retry: (count: number) => !!waiting || busyRef.current || count < 1,
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
  busyRef.current = isBusy(v?.upgrade);

  // 新版本已经在运行、升级的每一步都做完了：刷新页面，加载新的前端
  useEffect(() => {
    if (waiting && v?.current === waiting && !isBusy(v.upgrade)) {
      const t = setTimeout(() => window.location.reload(), 1500);
      return () => clearTimeout(t);
    }
  }, [waiting, v?.current, v?.upgrade]);
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
  const busy = !!waiting || isBusy(v.upgrade);
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
    const done = waiting !== null && v.current === waiting && !isBusy(u);
    return done ? (
      <p className="flex items-center gap-2 font-medium">
        <span className="size-3.5 animate-spin rounded-full border-2 border-zinc-300 border-t-accent" />
        已升级到 v{waiting}，正在刷新页面
      </p>
    ) : (
      <UpgradeProgress u={u} target={u.target ?? waiting} />
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
      {v.backup && <BackupLine b={v.backup} />}

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

/** 升级是否还在进行：已提交或正在执行 */
export function isBusy(u?: UpgradeStatus): boolean {
  return u?.state === "requested" || u?.state === "running";
}

/** 整体进度（0 到 1）：做完和跳过的步骤算满，进行中的按说明里的百分比（例如下载 45%）算一部分 */
export function progressOf(steps: UpgradeStep[]): number {
  if (steps.length === 0) return 0;
  const part = steps.reduce((n, s) => {
    if (s.state === "done" || s.state === "skipped") return n + 1;
    if (s.state === "running") {
      const m = (s.detail ?? "").match(/^(\d+)%/);
      return n + (m ? Number(m[1]) / 100 : 0.3);
    }
    return n;
  }, 0);
  return Math.min(1, part / steps.length);
}

const STEP_ICON: Record<string, { icon: string; className: string }> = {
  pending: { icon: "○", className: "text-zinc-400" },
  running: { icon: "", className: "font-medium" },
  done: { icon: "✓", className: "text-emerald-600 dark:text-emerald-400" },
  skipped: { icon: "–", className: "text-zinc-400" },
  failed: { icon: "✕", className: "text-red-600 dark:text-red-400" },
};

/** 升级进度：进度条和每一步的状态；升级助手还没接手（没有步骤）时只显示一句当前在做什么 */
function UpgradeProgress({ u, target }: { u: UpgradeStatus; target?: string | null }) {
  const steps = u.steps;
  const pct = progressOf(steps);
  // 状态文件 10 分钟没有更新：升级助手可能卡住了
  const stale = !!u.updated_at && Date.now() - new Date(u.updated_at).getTime() > 10 * 60_000;
  const title = target ? `正在升级到 v${target}` : "正在做部署迁移";
  return (
    <div className="flex flex-col gap-2">
      <p className="flex items-center gap-2 font-medium">
        <span className="size-3.5 animate-spin rounded-full border-2 border-zinc-300 border-t-accent" />
        {title}
        {steps.length > 0 && <span className="num text-xs text-zinc-500">{Math.round(pct * 100)}%</span>}
      </p>
      {steps.length > 0 ? (
        <>
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-zinc-100 dark:bg-zinc-800">
            <div className="h-full bg-accent transition-all duration-500" style={{ width: `${pct * 100}%` }} />
          </div>
          <ol className="flex flex-col gap-1 text-xs">
            {steps.map((s) => {
              const st = STEP_ICON[s.state] ?? STEP_ICON.pending;
              return (
                <li key={s.key} className={clsx("flex items-baseline gap-1.5", st?.className)}>
                  {s.state === "running" ? (
                    <span className="inline-block size-2.5 shrink-0 animate-spin rounded-full border-2 border-zinc-300 border-t-accent" />
                  ) : (
                    <span className="w-2.5 shrink-0 text-center">{st?.icon}</span>
                  )}
                  <span className="shrink-0 whitespace-nowrap">{s.label}</span>
                  {s.detail && <span className="min-w-0 break-words font-normal text-zinc-500">{s.detail}</span>}
                </li>
              );
            })}
          </ol>
        </>
      ) : (
        <p className="text-zinc-500">{u.message ?? "等待升级助手接手"}</p>
      )}
      {stale ? (
        <p className="text-xs text-amber-600">
          超过 10 分钟没有新进展。可以在部署目录执行 docker compose logs upgrader 看看升级助手在做什么。
        </p>
      ) : (
        <p className="text-xs text-zinc-400">
          升级时服务会重启，期间页面可能暂时打不开，进度会在重新连上后接着显示。完成后自动刷新；点别处收起，再点版本号还能看到进度。
        </p>
      )}
    </div>
  );
}

const backupTime = (b: BackupStatus) => (b.latest ? new Date(b.latest.at * 1000).toISOString() : null);

/** 版本号弹窗里的一行：最近一次备份（每日或升级前） */
function BackupLine({ b }: { b: BackupStatus }) {
  return (
    <div className={clsx("text-xs", b.stale || b.error ? "text-amber-700 dark:text-amber-400" : "text-zinc-500")}>
      <p>
        数据库备份：
        {b.latest
          ? `最近一次 ${fmtAgo(backupTime(b))}（${(b.latest.size / 2 ** 20).toFixed(0)} MB），共保留 ${b.count} 份`
          : "还没有"}
      </p>
      {b.error && <p className="mt-0.5">上次失败：{b.error}</p>}
      {b.latest && (
        <p className="mt-0.5 text-zinc-400">在部署目录的 backups/ 下；建议再复制一份到别的机器，见运维手册「备份」</p>
      )}
    </div>
  );
}

/** 管理员页头下的提醒：超过 36 小时没有成功的备份 */
export function BackupAlert() {
  const q = useVersion(undefined, { query: { staleTime: 60_000 } });
  const b = q.data?.backup;
  if (!b || !(b.stale || b.error)) return null;
  return (
    <p className="rounded-lg bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:bg-amber-500/10 dark:text-amber-300">
      数据库{b.latest ? `已经 ${fmtAgo(backupTime(b)).replace("前", "")}没有成功备份` : "还没有备份"}
      {b.error ? `，上次失败：${b.error}` : ""}。点页头的版本号看详情，或执行 docker compose logs upgrader。
    </p>
  );
}
