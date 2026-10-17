import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useNavigate } from "react-router";
import { getListAccountsQueryKey, useCreateAccount } from "../api/generated/accounts/accounts";
import type { AccountDetail } from "../api/generated/model";
import { PositionsEditor } from "../components/PositionsEditor";
import { buttonClass, Card, errorText, Field, inputClass, PageHeader } from "../components/ui";

/** 新建账户：① 名字 ② 现金和持仓（可以粘贴，也可以先跳过）。完成后马上生成第一份建议；选股和风控规则以后在设置里调 */
export function AccountNewPage() {
  const nav = useNavigate();
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const [created, setCreated] = useState<AccountDetail | null>(null);
  const create = useCreateAccount({
    mutation: {
      onSuccess: (d) => {
        qc.invalidateQueries({ queryKey: getListAccountsQueryKey() });
        setCreated(d);
      },
    },
  });
  const steps = ["起个名字", "现金和持仓"];
  const step = created ? 2 : 1;
  return (
    <>
      <PageHeader
        title="新建账户"
        desc="两步：起个名字、填现金和持仓。选股和风控规则先用稳健的默认值，以后在账户的「设置」里可以调整。"
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
              if (name.trim()) create.mutate({ data: { name: name.trim(), cash: 0 } });
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
