import { useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { getMeQueryKey, useChangePassword, useMe } from "../api/generated/auth/auth";
import { Badge, buttonClass, Card, errorText, Field, inputClass, PageHeader } from "../components/ui";

/** 本人的账号：改密码。改完后其他浏览器上的登录都要重新登录，这台浏览器不受影响 */
export function MePage() {
  const me = useMe({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const qc = useQueryClient();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [again, setAgain] = useState("");
  const [done, setDone] = useState(false);
  const change = useChangePassword({
    mutation: {
      onSuccess: (status) => {
        qc.setQueryData(getMeQueryKey(), status);
        setCurrent("");
        setNext("");
        setAgain("");
        setDone(true);
      },
    },
  });
  const mismatch = again !== "" && next !== again;
  const submit = (e: FormEvent) => {
    e.preventDefault();
    setDone(false);
    change.mutate({ data: { current_password: current, new_password: next } });
  };
  return (
    <>
      <PageHeader
        title={me.data?.username ?? "我的"}
        extra={
          me.data && (
            <Badge tone={me.data.role === "admin" ? "info" : "muted"}>
              {me.data.role === "admin" ? "管理员" : "普通用户"}
            </Badge>
          )
        }
      />
      <Card title="改密码">
        <form onSubmit={submit} className="flex max-w-sm flex-col gap-3">
          <Field label="现在的密码">
            <input
              className={inputClass}
              type="password"
              autoComplete="current-password"
              value={current}
              onChange={(e) => setCurrent(e.target.value)}
            />
          </Field>
          <Field label="新密码（至少 8 个字符）">
            <input
              className={inputClass}
              type="password"
              autoComplete="new-password"
              value={next}
              onChange={(e) => setNext(e.target.value)}
            />
          </Field>
          <Field label="再输一遍新密码">
            <input
              className={inputClass}
              type="password"
              autoComplete="new-password"
              value={again}
              onChange={(e) => setAgain(e.target.value)}
            />
          </Field>
          {mismatch && <p className="text-sm text-red-600">两次输入的新密码不一样</p>}
          {!!change.error && <p className="text-sm text-red-600">{errorText(change.error)}</p>}
          {done && <p className="text-sm text-emerald-600">已改好。其他浏览器上的登录需要用新密码重新登录。</p>}
          <button
            type="submit"
            className={buttonClass()}
            disabled={change.isPending || !current || next.length < 8 || next !== again}
          >
            {change.isPending ? "保存中…" : "保存"}
          </button>
        </form>
        <p className="mt-3 text-xs text-zinc-500">忘记密码时请管理员重置。</p>
      </Card>
    </>
  );
}
