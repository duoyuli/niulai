import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate } from "react-router";
import { getListAccountsQueryKey, useCreateAccount } from "../api/generated/accounts/accounts";
import type { AccountDetail, Preset } from "../api/generated/model";
import { PositionsEditor } from "../components/PositionsEditor";
import { buttonClass, Card, errorText, Field, inputClass, PageHeader } from "../components/ui";

/** 两种选股方式：按勾选策略的买卖点（均衡型规则），或多因子月度调仓 */
const KINDS: { preset: Preset; label: string; desc: string }[] = [
  {
    preset: "balanced",
    label: "按策略的买卖点",
    desc: "参考全部内置策略的买点，策略的卖出条件满足时卖出。风控规则先用均衡型，以后在设置里调。",
  },
  {
    preset: "factor",
    label: "多因子月度调仓",
    desc: "不看买卖信号：每月最后一个交易日按低波动、低特质波动、利润增速、盈利质量和小规模给全市场打分，分散持有排名前 30 只；刚建好时马上建仓。",
  },
];

/** 新建账户：① 名字和选股方式 ② 现金和持仓（可以粘贴，也可以先跳过）。完成后马上生成第一份建议；规则以后在设置里调 */
export function AccountNewPage() {
  const nav = useNavigate();
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const [preset, setPreset] = useState<Preset>("balanced");
  const [created, setCreated] = useState<AccountDetail | null>(null);
  const create = useCreateAccount({
    mutation: {
      onSuccess: (d) => {
        qc.invalidateQueries({ queryKey: getListAccountsQueryKey() });
        setCreated(d);
      },
    },
  });
  const steps = ["名字和选股方式", "现金和持仓"];
  const step = created ? 2 : 1;
  return (
    <>
      <PageHeader
        title="新建账户"
        desc="两步：起个名字并选选股方式、填现金和持仓。规则以后在账户的「设置」里可以调整。"
      />
      <ol className="flex gap-4 text-sm">
        {steps.map((s, i) => (
          <li key={s} className={i + 1 === step ? "font-semibold" : "text-zinc-400"}>
            {i + 1}. {s}
          </li>
        ))}
      </ol>
      {!created && (
        <Card>
          <form
            className="flex flex-wrap items-end gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              if (name.trim()) create.mutate({ data: { name: name.trim(), cash: 0, preset } });
            }}
          >
            <Field label="账户名，例如「华泰主账户」「妈妈的账户」">
              <input
                className={`${inputClass} w-72`}
                maxLength={30}
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
            </Field>
            <fieldset className="basis-full">
              <legend className="text-sm text-zinc-600 dark:text-zinc-400">选股方式</legend>
              <div className="mt-1 grid gap-2 sm:grid-cols-2">
                {KINDS.map((k) => (
                  <label
                    key={k.preset}
                    className="flex cursor-pointer gap-2 rounded-lg border border-zinc-200 p-3 text-sm has-[:checked]:border-accent dark:border-zinc-700"
                  >
                    <input
                      type="radio"
                      name="kind"
                      className="mt-1 shrink-0 self-start"
                      checked={preset === k.preset}
                      onChange={() => setPreset(k.preset)}
                    />
                    <span>
                      <span className="font-medium">{k.label}</span>
                      <span className="mt-0.5 block text-xs text-zinc-500">{k.desc}</span>
                    </span>
                  </label>
                ))}
              </div>
            </fieldset>
            <button type="submit" className={buttonClass()} disabled={!name.trim() || create.isPending}>
              {create.isPending ? "创建中…" : "下一步"}
            </button>
            {!!create.error && <p className="basis-full text-sm text-red-600">{errorText(create.error)}</p>}
          </form>
        </Card>
      )}
      {created && (
        <Card title={`「${created.summary.account.name}」现在有多少现金、哪些持仓`}>
          <PositionsEditor
            detail={created}
            saveLabel="完成，看建议"
            onDone={() => nav(`/accounts/${created.summary.account.account_id}`)}
          />
          <button
            type="button"
            className="mt-3 text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
            onClick={() => nav(`/accounts/${created.summary.account.account_id}`)}
          >
            先跳过，以后再填
          </button>
        </Card>
      )}
    </>
  );
}
