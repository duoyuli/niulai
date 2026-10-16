import { useEffect, useState } from "react";
import { Field, inputClass } from "./ui";

/** 快线 / 慢线周期输入：失焦或回车时提交，快线必须小于慢线 */
export function MaControls({
  fast,
  slow,
  onChange,
}: {
  fast: number;
  slow: number;
  onChange: (fast: number, slow: number) => void;
}) {
  const [f, setF] = useState(String(fast));
  const [s, setS] = useState(String(slow));
  useEffect(() => {
    setF(String(fast));
    setS(String(slow));
  }, [fast, slow]);
  const nf = Number(f);
  const ns = Number(s);
  const valid = Number.isInteger(nf) && Number.isInteger(ns) && nf >= 2 && ns <= 250 && nf < ns;
  const commit = () => {
    if (valid && (nf !== fast || ns !== slow)) onChange(nf, ns);
  };
  return (
    <div className="flex items-end gap-2">
      <Field label="快线" hint="短周期均线，默认 15 日">
        <input
          className={`${inputClass} num w-16`}
          inputMode="numeric"
          value={f}
          onChange={(e) => setF(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
        />
      </Field>
      <Field label="慢线" hint="长周期均线，默认 30 日">
        <input
          className={`${inputClass} num w-16`}
          inputMode="numeric"
          value={s}
          onChange={(e) => setS(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
        />
      </Field>
      {!valid && <span className="pb-2 text-xs text-red-500">快线需小于慢线，范围 2–250</span>}
    </div>
  );
}
