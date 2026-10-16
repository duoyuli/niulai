import { Link } from "react-router";
import { buttonClass } from "../components/ui";

export function NotFoundPage() {
  return (
    <div className="flex flex-col items-center gap-4 py-20 text-center">
      <p className="text-4xl font-semibold">404</p>
      <p className="text-sm text-zinc-500">页面不存在</p>
      <Link to="/" className={buttonClass("ghost")}>
        回到首页
      </Link>
    </div>
  );
}
