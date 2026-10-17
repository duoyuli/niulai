import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate } from "react-router";
import { getListAccountsQueryKey, useCreateAccount } from "../api/generated/accounts/accounts";
import type { AccountDetail, Style } from "../api/generated/model";
import { PositionsEditor } from "../components/PositionsEditor";
import { StyleCards } from "../components/StyleCards";
import { buttonClass, Card, errorText, Field, inputClass, PageHeader } from "../components/ui";

/** 新建账户：① 名字 ② 风格 ③ 现金和持仓（可以粘贴，也可以先跳过）。完成后马上生成第一份建议 */
export function AccountNewPage() {
  const nav = useNavigate();
  const qc = useQueryClient();
  const [step, setStep] = useState(1);
  const [name, setName] = useState("");
  const [style, setStyle] = useState<Style>("balanced");
  const [created, setCreated] = useState<AccountDetail | null>(null);
  const create = useCreateAccount({
    mutation: {
      onSuccess: (d) => {
        qc.invalidateQueries({ queryKey: getListAccountsQueryKey() });
        setCreated(d);
        setStep(3);
      },
    },
  });
  const steps = ["起个名字", "选一个风格", "现金和持仓"];
  return (
    <>
      <PageHeader title="新建账户" desc="三步：起个名字、选风格、填现金和持仓。以后随时可以改。" />
      <ol className="flex gap-4 text-sm">
        {steps.map((s, i) => (
          <li key={s} className={i + 1 === step ? "font-semibold" : "text-zinc-400"}>
            {i + 1}. {s}
          </li>
        ))}
      </ol>
      {step === 1 && (
        <Card>
          <form
            className="flex flex-wrap items-end gap-3"
            onSubmit={(e) => {
              e.preventDefault();
              if (name.trim()) setStep(2);
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
            <button type="submit" className={buttonClass()} disabled={!name.trim()}>
              下一步
            </button>
          </form>
        </Card>
      )}
      {step === 2 && (
        <Card>
          <p className="mb-3 text-sm text-zinc-600 dark:text-zinc-400">
            风格决定参考哪些选股策略、最多持几只、亏多少止损、市场不好时怎么办。拿不准就选「稳健」，以后可以改，也可以在设置里调整每一项。
          </p>
          <StyleCards value={style} onChange={setStyle} />
          <div className="mt-4 flex items-center gap-3">
            <button type="button" className={buttonClass("ghost")} onClick={() => setStep(1)}>
              上一步
            </button>
            <button
              type="button"
              className={buttonClass()}
              disabled={create.isPending}
              onClick={() => create.mutate({ data: { name: name.trim(), style, cash: 0 } })}
            >
              {create.isPending ? "创建中…" : "下一步"}
            </button>
            {!!create.error && <p className="text-sm text-red-600">{errorText(create.error)}</p>}
          </div>
        </Card>
      )}
      {step === 3 && created && (
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
