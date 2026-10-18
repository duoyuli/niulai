import { useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import {
  getGetNotifyQueryKey,
  getMeQueryKey,
  useChangePassword,
  useGetNotify,
  useMe,
  useSetNotify,
  useTestNotify,
} from "../api/generated/auth/auth";
import type { NotifySettings } from "../api/generated/model";
import { Badge, buttonClass, Card, errorText, Field, inputClass, PageHeader } from "../components/ui";

/** 本人的账号：改密码、推送设置。改完密码后其他浏览器上的登录都要重新登录，这台浏览器不受影响 */
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
      <NotifyCard />
    </>
  );
}

/** 推送：填一个群机器人地址，每个交易日的运营建议生成后发过去 */
function NotifyCard() {
  const q = useGetNotify();
  return <Card title="推送运营建议">{q.data ? <NotifyForm cur={q.data} /> : null}</Card>;
}

function NotifyForm({ cur }: { cur: NotifySettings }) {
  const qc = useQueryClient();
  const [url, setUrl] = useState(cur.webhook ?? "");
  const [sent, setSent] = useState(false);
  const onSaved = (v: NotifySettings) => {
    qc.setQueryData(getGetNotifyQueryKey(), v);
    setSent(false);
  };
  const save = useSetNotify({ mutation: { onSuccess: onSaved } });
  const test = useTestNotify({ mutation: { onSuccess: () => setSent(true) } });
  const dirty = url.trim() !== (cur.webhook ?? "");
  return (
    <div className="flex max-w-xl flex-col gap-3 text-sm">
      <p className="text-zinc-600 dark:text-zinc-400">
        每个交易日收盘、数据同步完生成运营建议后，把要卖什么、要买什么、最高买入价发到你的群聊里，不用记着打开网页。支持企业微信、飞书、钉钉的群机器人：在群设置里添加「自定义机器人」，复制它的地址填在下面（钉钉的安全设置选「自定义关键词」，填「牛来」）。
      </p>
      <Field label={`群机器人地址${cur.channel ? `（${cur.channel}）` : ""}`}>
        <input
          className={inputClass}
          placeholder="https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=…"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
        />
      </Field>
      <div className="flex flex-wrap items-center gap-3">
        <button
          type="button"
          className={buttonClass()}
          disabled={!dirty || save.isPending}
          onClick={() => save.mutate({ data: { webhook: url.trim() || null } })}
        >
          {save.isPending ? "保存中…" : url.trim() ? "保存" : "不再推送"}
        </button>
        {cur.webhook && !dirty && (
          <button
            type="button"
            className={buttonClass("ghost")}
            disabled={test.isPending}
            onClick={() => test.mutate()}
          >
            {test.isPending ? "发送中…" : "发一条试试"}
          </button>
        )}
        {sent && <span className="text-emerald-600">已发出，去群里看看</span>}
      </div>
      {!!save.error && <p className="text-red-600">{errorText(save.error)}</p>}
      {!!test.error && <p className="text-red-600">{errorText(test.error)}</p>}
      {cur.last_error && <p className="text-amber-700 dark:text-amber-400">上次推送失败：{cur.last_error}</p>}
    </div>
  );
}
