import { isRouteErrorResponse, useRouteError } from "react-router";
import { buttonClass, Card } from "./ui";

/** 页面渲染出错时显示：说明发生了什么，给一个刷新按钮。发布新版本后旧页面去加载已经不存在的分包，刷新就好 */
export function RouteError() {
  const error = useRouteError();
  const stale =
    error instanceof Error && /dynamically imported module|Importing a module script failed/i.test(error.message);
  const detail = isRouteErrorResponse(error)
    ? `${error.status} ${error.statusText}`
    : error instanceof Error
      ? error.message
      : String(error);
  return (
    <Card>
      <h2 className="text-lg font-semibold">{stale ? "系统刚更新过，请刷新页面" : "这个页面出错了"}</h2>
      <p className="mt-2 text-sm text-zinc-500">
        {stale ? "页面还是旧版本，刷新后会加载新版本。" : "可以先刷新试试；一直出错的话，把下面的信息发给管理员。"}
      </p>
      {!stale && <pre className="mt-2 overflow-auto rounded bg-zinc-100 p-2 text-xs dark:bg-zinc-800">{detail}</pre>}
      <button type="button" className={`${buttonClass()} mt-3`} onClick={() => window.location.reload()}>
        刷新
      </button>
    </Card>
  );
}
