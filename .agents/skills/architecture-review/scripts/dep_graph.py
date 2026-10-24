#!/usr/bin/env python3
"""统计源码目录中顶层模块之间的静态依赖。

输出模块的扇入（Ca）、扇出（Ce）、不稳定度 I = Ce / (Ca + Ce)、依赖边（引用次数和示例位置）、
依赖环（强连通分量）以及违反稳定依赖方向的边。只做静态文本分析，结果是线索，需要打开代码确认。

用法：
  dep_graph.py --lang rust backend/src
  dep_graph.py --lang ts frontend/src --depth 2
  dep_graph.py --lang rust backend/src --json
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = "(root)"

RUST_PATH = re.compile(r"\bcrate::([A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*)")
RUST_GROUP = re.compile(r"\bcrate::\{")
RUST_TEST_MOD = re.compile(r"^\s*#\[cfg\(test\)\]\s*\n\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{", re.M)
TS_TEST_FILE = re.compile(r"\.(test|spec)\.[cm]?[jt]sx?$")
TS_IMPORT = re.compile(
    r"""(?:\bfrom\s*|\bimport\s*\(\s*|\bimport\s+)['"]([^'"]+)['"]"""
)


def module_of(rel_parts: tuple[str, ...], depth: int, strip_ext: bool) -> str:
    """把相对 src 的路径映射到模块名，取前 depth 段。"""
    parts = list(rel_parts)
    if strip_ext and parts:
        stem = parts[-1].rsplit(".", 1)[0]
        parts[-1] = stem
    if len(parts) == 1 and parts[0] in ("lib", "main", "mod", "index"):
        return ROOT
    return "/".join(parts[:depth])


def strip_line_comment(line: str) -> str:
    stripped = line.lstrip()
    if stripped.startswith("//"):
        return ""
    return line


def split_top_level(body: str) -> list[str]:
    items, depth, cur = [], 0, []
    for ch in body:
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
        if ch == "," and depth == 0:
            items.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    items.append("".join(cur))
    return [i.strip() for i in items if i.strip()]


def rust_refs(text: str) -> list[tuple[int, list[str]]]:
    """返回 (行号, crate:: 之后的路径段) 列表，展开 use crate::{a, b::c}。"""
    lines = [strip_line_comment(line) for line in text.splitlines()]
    clean = "\n".join(lines)
    refs: list[tuple[int, list[str]]] = []
    for m in RUST_PATH.finditer(clean):
        line_no = clean.count("\n", 0, m.start()) + 1
        refs.append((line_no, m.group(1).split("::")))
    for m in RUST_GROUP.finditer(clean):
        start = m.end()
        depth, i = 1, start
        while i < len(clean) and depth:
            depth += {"{": 1, "}": -1}.get(clean[i], 0)
            i += 1
        line_no = clean.count("\n", 0, m.start()) + 1
        for item in split_top_level(clean[start : i - 1]):
            name = re.match(r"[A-Za-z_][A-Za-z0-9_]*", item)
            if name and name.group(0) != "self":
                rest = re.findall(r"[A-Za-z_][A-Za-z0-9_]*", item.split("{")[0])
                refs.append((line_no, rest))
    return refs


def collect_rust(src: Path, depth: int, include_tests: bool):
    files = [
        p for p in src.rglob("*.rs")
        if "target" not in p.parts and (include_tests or "tests" not in p.relative_to(src).parts)
    ]
    known_dirs = {p.name for p in src.iterdir() if p.is_dir()}
    edges: dict[tuple[str, str], list[str]] = defaultdict(list)
    modules: dict[str, int] = defaultdict(int)
    for path in files:
        rel = path.relative_to(src).parts
        if rel[0] == "bin" or (len(rel) == 1 and rel[0] in ("lib.rs", "main.rs")):
            mod = ROOT
        else:
            mod = module_of(rel, depth, strip_ext=True)
            if len(rel) >= 2 and rel[-1] == "mod.rs" and depth >= len(rel) - 1:
                mod = "/".join(rel[:-1][:depth])
        modules[mod] += 1
        text = path.read_text(encoding="utf-8", errors="replace")
        if not include_tests:
            # 内联测试模块按惯例位于文件末尾，从它开始截断
            m = RUST_TEST_MOD.search(text)
            if m:
                text = text[: m.start()]
        for line_no, segs in rust_refs(text):
            if not segs:
                continue
            target_parts = []
            for seg in segs[:depth]:
                target_parts.append(seg)
            # 只有目录型模块才能继续向下取子模块
            if depth >= 2 and segs[0] not in known_dirs:
                target_parts = segs[:1]
            target = "/".join(target_parts)
            if target and target != mod and not mod.startswith(target + "/") and not target.startswith(mod + "/"):
                edges[(mod, target)].append(f"{path.relative_to(src)}:{line_no}")
    # 目标中只保留确实存在的模块（排除引用的是 crate 根下的条目，例如 crate::Error）
    existing = set(modules)
    edges = {k: v for k, v in edges.items() if k[1] in existing}
    return dict(modules), edges


def resolve_ts(spec: str, file: Path, src: Path) -> Path | None:
    if spec.startswith("@/"):
        return src / spec[2:]
    if spec.startswith("~/"):
        return src / spec[2:]
    if spec.startswith("."):
        return (file.parent / spec).resolve()
    return None


def collect_ts(src: Path, depth: int, include_tests: bool):
    src = src.resolve()
    exts = {".ts", ".tsx", ".js", ".jsx", ".mts", ".cts"}
    files = [
        p for p in src.rglob("*")
        if p.suffix in exts and "node_modules" not in p.parts and p.is_file()
        and (include_tests or not (TS_TEST_FILE.search(p.name) or "__tests__" in p.parts))
    ]
    edges: dict[tuple[str, str], list[str]] = defaultdict(list)
    modules: dict[str, int] = defaultdict(int)

    def mod_for(path: Path) -> str | None:
        try:
            rel = path.relative_to(src).parts
        except ValueError:
            return None
        if len(rel) == 1:
            return ROOT
        return "/".join(rel[: min(depth, len(rel) - 1)])

    for path in files:
        mod = mod_for(path)
        modules[mod] += 1
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in TS_IMPORT.finditer(text):
            target_path = resolve_ts(m.group(1), path, src)
            if target_path is None:
                continue
            target = mod_for(target_path)
            if target is None or target == mod:
                continue
            line_no = text.count("\n", 0, m.start()) + 1
            edges[(mod, target)].append(f"{path.relative_to(src)}:{line_no}")
    return dict(modules), dict(edges)


def tarjan(nodes, adj):
    index, low, stack, on, out, counter = {}, {}, [], set(), [], [0]
    sys.setrecursionlimit(10000)

    def visit(v):
        index[v] = low[v] = counter[0]
        counter[0] += 1
        stack.append(v)
        on.add(v)
        for w in adj.get(v, ()):
            if w not in index:
                visit(w)
                low[v] = min(low[v], low[w])
            elif w in on:
                low[v] = min(low[v], index[w])
        if low[v] == index[v]:
            comp = []
            while True:
                w = stack.pop()
                on.discard(w)
                comp.append(w)
                if w == v:
                    break
            if len(comp) > 1:
                out.append(sorted(comp))

    for v in sorted(nodes):
        if v not in index:
            visit(v)
    return out


def analyze(modules, edges, include_root: bool):
    if not include_root:
        edges = {k: v for k, v in edges.items() if ROOT not in k}
    nodes = sorted(m for m in modules if include_root or m != ROOT)
    adj = defaultdict(set)
    ca, ce = defaultdict(set), defaultdict(set)
    for (a, b) in edges:
        adj[a].add(b)
        ce[a].add(b)
        ca[b].add(a)
    metrics = {}
    for n in nodes:
        a, e = len(ca[n]), len(ce[n])
        metrics[n] = {
            "files": modules.get(n, 0),
            "ca": a,
            "ce": e,
            "instability": round(e / (a + e), 2) if a + e else None,
        }
    cycles = tarjan(nodes, adj)
    sdp = []
    for (a, b), locs in edges.items():
        ia, ib = metrics[a]["instability"], metrics[b]["instability"]
        if ia is not None and ib is not None and ib > ia:
            sdp.append({"from": a, "to": b, "i_from": ia, "i_to": ib, "count": len(locs)})
    edge_list = [
        {"from": a, "to": b, "count": len(locs), "examples": locs[:3]}
        for (a, b), locs in sorted(edges.items(), key=lambda kv: (kv[0][0], -len(kv[1])))
    ]
    cycle_edges = []
    for comp in cycles:
        s = set(comp)
        cycle_edges.append([e for e in edge_list if e["from"] in s and e["to"] in s])
    return {
        "modules": metrics,
        "edges": edge_list,
        "cycles": [{"modules": c, "edges": ce_} for c, ce_ in zip(cycles, cycle_edges)],
        "sdp_violations": sorted(sdp, key=lambda d: d["i_to"] - d["i_from"], reverse=True),
    }


def render_md(result) -> str:
    out = ["## 模块", "", "| 模块 | 文件数 | 扇入 Ca | 扇出 Ce | 不稳定度 I |", "| --- | --- | --- | --- | --- |"]
    for name, m in sorted(result["modules"].items(), key=lambda kv: (kv[1]["instability"] is None, kv[1]["instability"] or 0)):
        i = "-" if m["instability"] is None else f'{m["instability"]:.2f}'
        out.append(f'| {name} | {m["files"]} | {m["ca"]} | {m["ce"]} | {i} |')
    out += ["", "## 依赖边", "", "| 从 | 到 | 引用次数 | 示例位置 |", "| --- | --- | --- | --- |"]
    for e in result["edges"]:
        out.append(f'| {e["from"]} | {e["to"]} | {e["count"]} | {", ".join(e["examples"])} |')
    out += ["", "## 依赖环", ""]
    if not result["cycles"]:
        out.append("无。")
    for c in result["cycles"]:
        out.append(f'- 环：{" ↔ ".join(c["modules"])}')
        for e in sorted(c["edges"], key=lambda e: e["count"]):
            out.append(f'  - {e["from"]} → {e["to"]}（{e["count"]} 处，例如 {e["examples"][0]}）')
    out += ["", "## 违反稳定依赖方向的边（从更稳定指向更不稳定）", ""]
    if not result["sdp_violations"]:
        out.append("无。")
    for v in result["sdp_violations"]:
        out.append(f'- {v["from"]}（I={v["i_from"]}）→ {v["to"]}（I={v["i_to"]}），{v["count"]} 处')
    return "\n".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("src", type=Path, help="源码根目录，例如 backend/src 或 frontend/src")
    parser.add_argument("--lang", choices=["rust", "ts"], required=True)
    parser.add_argument("--depth", type=int, default=1, help="按几层目录划分模块，默认 1（顶层模块）")
    parser.add_argument("--include-root", action="store_true", help="统计 lib.rs、main.rs、bin/、入口文件等根节点")
    parser.add_argument("--include-tests", action="store_true", help="统计测试代码（默认排除测试文件、tests/ 目录和末尾的 #[cfg(test)] 模块）")
    parser.add_argument("--json", action="store_true", help="输出 JSON")
    args = parser.parse_args()
    if not args.src.is_dir():
        print(f"不是目录：{args.src}", file=sys.stderr)
        return 2
    collect = collect_rust if args.lang == "rust" else collect_ts
    modules, edges = collect(args.src, max(1, args.depth), args.include_tests)
    result = analyze(modules, edges, args.include_root)
    print(json.dumps(result, ensure_ascii=False, indent=2) if args.json else render_md(result))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
