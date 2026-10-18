/** 把表格导成 CSV 下载。带 UTF-8 BOM，Excel 直接打开中文不乱码；数字按原样写。 */

type Cell = string | number | null | undefined;

/** 单元格：有逗号、引号或换行时加引号，引号本身双写 */
function cell(v: Cell): string {
  const s = v === null || v === undefined ? "" : String(v);
  return /[",\r\n]/.test(s) ? `"${s.replaceAll('"', '""')}"` : s;
}

/** 生成 CSV 并触发下载。filename 不含 .csv，自动补上 */
export function downloadCsv(filename: string, headers: string[], rows: Cell[][]) {
  const text = [headers, ...rows].map((r) => r.map(cell).join(",")).join("\r\n");
  const blob = new Blob(["\ufeff", text], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename.endsWith(".csv") ? filename : `${filename}.csv`;
  a.click();
  // 下载开始后再释放；立即释放有些浏览器会拿不到内容
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
