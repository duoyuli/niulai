import { useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { useMe } from "../api/generated/auth/auth";
import { type Role, Role as Roles, type User } from "../api/generated/model";
import {
  getListUsersQueryKey,
  useAddUser,
  useListUsers,
  useResetPassword,
  useUpdateUser,
} from "../api/generated/users/users";
import { Badge, buttonClass, Card, errorText, Field, inputClass, PageHeader, QueryView, Table } from "../components/ui";
import { fmtDateTime } from "../lib/format";

const ROLE_NAMES: Record<Role, string> = { admin: "管理员", member: "普通用户" };

/** 用户管理（管理员）：不开放注册，用户都在这里添加；可以设为管理员、重置密码、停用 */
export function UsersPage() {
  const list = useListUsers();
  // 系统生成的密码只显示这一次：交给本人，登录后提醒修改
  const [shown, setShown] = useState<{ username: string; password: string } | null>(null);
  return (
    <>
      <PageHeader
        title="用户"
        desc="每个人用自己的账号登录，自选股和以后的账户只有本人看得到，管理员也看不到。普通用户可以查看行情和因子研究；数据同步、数据源设置和用户管理只有管理员能用。"
      />
      {shown && (
        <p className="rounded-lg bg-emerald-50 px-3 py-2 text-sm text-emerald-800 dark:bg-emerald-500/10 dark:text-emerald-300">
          「{shown.username}」的密码是 <code className="num font-mono font-semibold select-all">{shown.password}</code>
          ，只显示这一次，请转交本人。本人登录后会被提醒改成自己的密码。
          <button type="button" className="ml-2 underline" onClick={() => setShown(null)}>
            知道了
          </button>
        </p>
      )}
      <AddUser onAdded={(username, password) => password && setShown({ username, password })} />
      <Card title="全部用户">
        <QueryView query={list}>
          {(users) => (
            <Table>
              <thead>
                <tr>
                  <th>用户名</th>
                  <th>角色</th>
                  <th>状态</th>
                  <th>添加时间</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {users.map((u) => (
                  <UserRow key={u.user_id} u={u} onReset={(password) => setShown({ username: u.username, password })} />
                ))}
              </tbody>
            </Table>
          )}
        </QueryView>
      </Card>
    </>
  );
}

function AddUser({ onAdded }: { onAdded: (username: string, password?: string | null) => void }) {
  const qc = useQueryClient();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [role, setRole] = useState<Role>(Roles.member);
  const add = useAddUser({
    mutation: {
      onSuccess: (r) => {
        qc.invalidateQueries({ queryKey: getListUsersQueryKey() });
        onAdded(r.user.username, r.generated_password);
        setUsername("");
        setPassword("");
        setRole(Roles.member);
      },
    },
  });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    add.mutate({ data: { username: username.trim(), password: password || null, role } });
  };
  return (
    <Card title="添加用户">
      <form onSubmit={submit} className="flex flex-wrap items-end gap-3">
        <Field label="用户名">
          <input className={inputClass} value={username} onChange={(e) => setUsername(e.target.value)} />
        </Field>
        <Field label="初始密码（不填时系统生成一个）">
          <input
            className={inputClass}
            type="text"
            autoComplete="off"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </Field>
        <Field label="角色" hint="管理员可以管理用户、数据同步和数据源设置；没有必要时选普通用户">
          <select className={inputClass} value={role} onChange={(e) => setRole(e.target.value as Role)}>
            <option value={Roles.member}>{ROLE_NAMES.member}</option>
            <option value={Roles.admin}>{ROLE_NAMES.admin}</option>
          </select>
        </Field>
        <button
          type="submit"
          className={buttonClass()}
          disabled={add.isPending || !username.trim() || (password !== "" && password.length < 8)}
        >
          {add.isPending ? "添加中…" : "添加"}
        </button>
      </form>
      {password !== "" && password.length < 8 && <p className="mt-2 text-sm text-zinc-500">密码至少 8 个字符</p>}
      {!!add.error && <p className="mt-2 text-sm text-red-600">{errorText(add.error)}</p>}
    </Card>
  );
}

function UserRow({ u, onReset }: { u: User; onReset: (password: string) => void }) {
  const qc = useQueryClient();
  const me = useMe({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const self = me.data?.user_id === u.user_id;
  const refresh = () => qc.invalidateQueries({ queryKey: getListUsersQueryKey() });
  const update = useUpdateUser({ mutation: { onSuccess: refresh } });
  const reset = useResetPassword({
    mutation: {
      onSuccess: (r) => {
        refresh();
        if (r.generated_password) onReset(r.generated_password);
      },
    },
  });
  const set = (patch: { role?: Role; disabled?: boolean }) =>
    update.mutate({ userId: u.user_id, data: { role: patch.role ?? u.role, disabled: patch.disabled ?? u.disabled } });
  const other = u.role === Roles.admin ? Roles.member : Roles.admin;
  return (
    <tr className={u.disabled ? "text-zinc-400" : undefined}>
      <td>
        {u.username}
        {self && <span className="ml-1 text-xs text-zinc-400">（我）</span>}
      </td>
      <td>{u.role === Roles.admin ? <Badge tone="info">管理员</Badge> : ROLE_NAMES.member}</td>
      <td>
        {u.disabled ? (
          <Badge>已停用</Badge>
        ) : u.must_change_password ? (
          <span className="text-xs text-zinc-500">还没改初始密码</span>
        ) : (
          "正常"
        )}
      </td>
      <td className="num text-xs">{fmtDateTime(u.created_at)}</td>
      <td>
        {self ? (
          <span className="text-xs text-zinc-400">自己的密码在「我的」页面改</span>
        ) : (
          <div className="flex flex-wrap items-center gap-3 text-xs">
            <button
              type="button"
              className="text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
              disabled={update.isPending}
              onClick={() => {
                if (window.confirm(`把「${u.username}」改为${ROLE_NAMES[other]}？`)) set({ role: other });
              }}
            >
              设为{ROLE_NAMES[other]}
            </button>
            <button
              type="button"
              className="text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
              disabled={reset.isPending}
              onClick={() => {
                if (window.confirm(`重置「${u.username}」的密码？这个用户已登录的浏览器都要用新密码重新登录。`))
                  reset.mutate({ userId: u.user_id, data: { password: null } });
              }}
            >
              重置密码
            </button>
            <button
              type="button"
              className={
                u.disabled
                  ? "text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
                  : "text-zinc-500 hover:text-red-600"
              }
              disabled={update.isPending}
              onClick={() => {
                if (
                  u.disabled ||
                  window.confirm(`停用「${u.username}」？停用后不能登录，已登录的立即退出；数据保留，可以再启用。`)
                )
                  set({ disabled: !u.disabled });
              }}
            >
              {u.disabled ? "启用" : "停用"}
            </button>
          </div>
        )}
        {!!(update.error || reset.error) && (
          <p className="mt-1 text-xs text-red-600">{errorText(update.error ?? reset.error)}</p>
        )}
      </td>
    </tr>
  );
}
