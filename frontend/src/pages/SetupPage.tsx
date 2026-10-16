import { useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Link } from "react-router";
import type { SetupStatus } from "../api/generated/model";
import { getGetSetupQueryKey, useGetSetup, usePutApiKey } from "../api/generated/setup/setup";
import { Badge, buttonClass, Card, errorText, inputClass, PageHeader, QueryView } from "../components/ui";

/** 首次登录引导：配置同花顺金融数据服务的 API Key。也用于以后更换 Key */
export function SetupPage() {
  const query = useGetSetup();
  return <QueryView query={query}>{(data) => <Setup status={data} />}</QueryView>;
}

function Setup({ status }: { status: SetupStatus }) {
  const [saved, setSaved] = useState<{ backfill: boolean } | null>(null);
  if (saved) return <Done backfill={saved.backfill} />;
  const first = !status.api_key_configured;
  return (
    <>
      <PageHeader
        title={first ? "欢迎使用牛来" : "数据源设置"}
        desc={
          first
            ? "牛来的行情、财务等数据来自同花顺金融数据服务。开始之前，需要配置一个 API Key，用来向它请求数据。免费 Key 就够用。"
            : "牛来通过同花顺金融数据服务的 API Key 请求数据。"
        }
      />
      {!first && (
        <Card title="当前的 API Key">
          <div className="flex flex-wrap items-center gap-3 text-sm">
            <span className="num font-mono">{status.api_key_hint}</span>
            {status.api_key_source === "env" ? <Badge>由部署配置提供</Badge> : <Badge tone="ok">在网页上配置</Badge>}
          </div>
          {status.api_key_source === "env" && (
            <p className="mt-3 text-sm text-zinc-500">
              这个 Key 来自环境变量 <code className="font-mono">HITHINK_FINANCE_API_KEY</code>
              ，不能在网页上修改。要更换，修改部署配置（<code className="font-mono">deploy/.env</code>）后重启服务。
            </p>
          )}
        </Card>
      )}
      {status.api_key_source !== "env" && (
        <Card title={first ? "配置 API Key" : "更换 API Key"}>
          <ol className="mb-5 list-decimal space-y-2 pl-5 text-sm text-zinc-600 dark:text-zinc-400">
            <li>
              打开同花顺金融数据服务的管理页：{" "}
              <a href={status.api_key_url} target="_blank" rel="noreferrer" className="text-accent hover:underline">
                {status.api_key_url} ↗
              </a>
              ，登录或注册账号。
            </li>
            <li>在管理页中创建一个 API Key，并复制它。</li>
            <li>粘贴到下面，点「验证并保存」。保存前会用它实际请求一次数据，确认可用。</li>
          </ol>
          <KeyForm first={first} backfillPlanned={status.backfill_planned} onSaved={setSaved} />
          <p className="mt-4 text-xs text-zinc-400">
            Key 保存在本服务的数据库中，只用于向同花顺请求数据，页面上只显示末尾 4 位。同一个 Key
            不要同时用在两套牛来上。
          </p>
        </Card>
      )}
    </>
  );
}

function KeyForm({
  first,
  backfillPlanned,
  onSaved,
}: {
  first: boolean;
  backfillPlanned: boolean;
  onSaved: (r: { backfill: boolean }) => void;
}) {
  const qc = useQueryClient();
  const [key, setKey] = useState("");
  const [show, setShow] = useState(false);
  const save = usePutApiKey({
    mutation: {
      onSuccess: (r) => {
        qc.invalidateQueries({ queryKey: getGetSetupQueryKey() });
        onSaved({ backfill: r.backfill_run_id !== null && r.backfill_run_id !== undefined });
      },
    },
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (key.trim()) save.mutate({ data: { api_key: key } });
  };
  return (
    <form onSubmit={submit} className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <input
          className={`${inputClass} w-full max-w-md font-mono`}
          type={show ? "text" : "password"}
          autoComplete="off"
          spellCheck={false}
          placeholder="粘贴 API Key"
          aria-label="API Key"
          value={key}
          onChange={(e) => setKey(e.target.value)}
        />
        <button type="button" className={buttonClass("ghost")} onClick={() => setShow((v) => !v)}>
          {show ? "隐藏" : "显示"}
        </button>
      </div>
      {save.error && <p className="text-sm text-red-600 dark:text-red-400">{errorText(save.error)}</p>}
      <div className="flex flex-wrap items-center gap-3">
        <button type="submit" className={buttonClass()} disabled={!key.trim() || save.isPending}>
          {save.isPending ? "验证中…" : "验证并保存"}
        </button>
        {first && !backfillPlanned && <span className="text-xs text-zinc-500">保存后会自动开始拉取全部历史数据。</span>}
      </div>
    </form>
  );
}

function Done({ backfill }: { backfill: boolean }) {
  return (
    <>
      <PageHeader title="API Key 已保存" desc="已经生效，同步任务会用这个 Key 请求数据。" />
      <Card>
        {backfill ? (
          <p className="text-sm">
            已开始首次全量回补：拉取 2005 年以来的日线、公司行动、财务数据等，大约需要 51 小时。期间可以正常使用页面，
            数据会陆续出现；每个交易日收盘后的同步也会照常进行。
          </p>
        ) : (
          <p className="text-sm">之后的同步任务都会使用新的 Key。</p>
        )}
        <div className="mt-4 flex flex-wrap gap-3">
          <Link to="/sync" className={buttonClass()}>
            查看同步进度
          </Link>
          <Link to="/" className={buttonClass("ghost")}>
            去看股票
          </Link>
        </div>
      </Card>
    </>
  );
}
