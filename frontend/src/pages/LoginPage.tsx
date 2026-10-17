import { type FormEvent, useState } from "react";
import { useSearchParams } from "react-router";
import { useLogin } from "../api/generated/auth/auth";
import { buttonClass, errorText, inputClass } from "../components/ui";

/** 登录页：用管理员添加的账号登录（不开放注册），登录状态在浏览器保存 30 天 */
export function LoginPage() {
  const [params] = useSearchParams();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const login = useLogin({
    mutation: {
      // 整页跳转：清掉登录前失败的请求缓存
      onSuccess: () => {
        const next = params.get("next");
        window.location.assign(next?.startsWith("/") && !next.startsWith("//") ? next : "/");
      },
    },
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    login.mutate({ data: { username: username.trim(), password } });
  };
  return (
    <div className="flex min-h-dvh items-center justify-center px-4">
      <form
        onSubmit={submit}
        className="flex w-full max-w-sm flex-col gap-4 rounded-xl border border-zinc-200 bg-white p-6 dark:border-zinc-800 dark:bg-zinc-900"
      >
        <div className="flex items-center gap-2">
          <img src="/favicon.svg" alt="" className="size-7" />
          <h1 className="text-lg font-semibold">登录牛来</h1>
        </div>
        <label className="flex flex-col gap-1 text-xs text-zinc-500">
          账号
          <input
            className={inputClass}
            autoComplete="username"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
          />
        </label>
        <label className="flex flex-col gap-1 text-xs text-zinc-500">
          密码
          <input
            className={inputClass}
            type="password"
            autoComplete="current-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
        {login.error && <p className="text-sm text-red-600">{errorText(login.error)}</p>}
        <button type="submit" className={buttonClass()} disabled={login.isPending || !username || !password}>
          {login.isPending ? "登录中…" : "登录"}
        </button>
        <p className="text-xs text-zinc-400">登录后这台浏览器 30 天内不用再登录。没有账号或忘记密码，请找管理员。</p>
      </form>
    </div>
  );
}
