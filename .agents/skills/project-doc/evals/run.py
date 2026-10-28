#!/usr/bin/env python3
"""project-doc 评测运行器。

用法（在仓库根目录）：
  uv run --with pyyaml==6.0.2 python .agents/skills/project-doc/evals/run.py \
      --arm baseline=none --arm current=<变体目录> --runs 3 --out <结果目录>

变体目录包含 skill/（整个 project-doc 目录，不含 evals/）和 gate.md（AGENTS.md 中
「## Project Doc gate」到文件末尾的内容）。arm 为 none 时删除 skill 和 gate。

每个用例在独立的 `git clone --shared` 中运行：检出 case.yaml 的 base，安装 arm，执行
setup.sh 并提交，然后把 source.apply 提交里 docs/ 以外的改动作为未提交改动放进工作区，
最后用 `claude -p` 执行 prompt.md。grader 见 graders/*.md 的 frontmatter。
"""
import argparse, concurrent.futures as cf, json, os, re, shutil, statistics, subprocess, sys, time
from pathlib import Path

import yaml

EVALS = Path(__file__).resolve().parent
REPO = EVALS.parents[3]
CHECKER = EVALS.parent / 'scripts' / 'check_library.py'
GATE_HEAD = '## Project Doc gate'
LIMIT = re.compile(r"hit your (session|usage) limit|usage limit", re.I)


def sh(cmd, cwd=None, check=True, **kw):
    r = subprocess.run(cmd, cwd=cwd, shell=isinstance(cmd, str), capture_output=True, text=True, **kw)
    if check and r.returncode:
        raise RuntimeError(f'{cmd}\n{r.stdout}\n{r.stderr}')
    return r


def front(path):
    text = Path(path).read_text()
    m = re.match(r'^---\n(.*?)\n---\n?(.*)$', text, re.S)
    return (yaml.safe_load(m.group(1)) or {}, m.group(2).strip()) if m else ({}, text.strip())


def load_cases(pattern):
    cases = []
    for d in sorted(p for p in EVALS.iterdir() if (p / 'case.yaml').exists()):
        if pattern and not re.search(pattern, d.name):
            continue
        case = yaml.safe_load((d / 'case.yaml').read_text())
        meta, prompt = front(d / 'prompt.md')
        graders = [dict(front(g)[0], name=g.stem, body=front(g)[1]) for g in sorted((d / 'graders').glob('*.md'))]
        cases.append(dict(case, dir=d, meta=meta, prompt=prompt, graders=graders))
    return cases


# ---------- 工作区 ----------

def install_arm(ws, arm):
    skill = ws / '.agents/skills/project-doc'
    shutil.rmtree(skill, ignore_errors=True)
    claude_skills = ws / '.claude/skills'
    if claude_skills.exists() and not claude_skills.is_symlink():
        link = claude_skills / 'project-doc'
        if link.is_symlink() or link.exists():
            shutil.rmtree(link) if link.is_dir() and not link.is_symlink() else link.unlink()
    agents = ws / 'AGENTS.md'
    text = agents.read_text()
    head = text.split(GATE_HEAD)[0].rstrip() + '\n'
    if arm['spec'] != 'none':
        src = Path(arm['spec'])
        shutil.copytree(src / 'skill', skill, ignore=shutil.ignore_patterns('evals', '__pycache__'))
        if claude_skills.exists() and not claude_skills.is_symlink():
            (claude_skills / 'project-doc').symlink_to('../../.agents/skills/project-doc')
        head += '\n' + (src / 'gate.md').read_text().strip() + '\n'
    agents.write_text(head)
    claude_md = ws / 'CLAUDE.md'
    if not claude_md.is_symlink():
        claude_md.unlink(missing_ok=True)
        claude_md.symlink_to('AGENTS.md')


def prepare(case, arm, ws):
    src = case['source']
    sh(['git', 'clone', '-q', '--shared', '--no-checkout', str(REPO), str(ws)])
    sh(['git', 'checkout', '-q', '--detach', src['base']], cwd=ws)
    sh(['git', 'remote', 'remove', 'origin'], cwd=ws)
    sh(['git', 'config', 'user.email', 'eval@example.invalid'], cwd=ws)
    sh(['git', 'config', 'user.name', 'eval'], cwd=ws)
    install_arm(ws, arm)
    if (case['dir'] / 'setup.sh').exists():
        sh(['bash', str(case['dir'] / 'setup.sh')], cwd=ws)
    sh('git add -A && git commit -q -m "chore: local setup" --no-verify', cwd=ws)
    setup_ref = sh(['git', 'rev-parse', 'HEAD'], cwd=ws).stdout.strip()
    if src.get('apply'):
        patch = sh(['git', 'diff', src['base'], src['apply'], '--', '.', ':(exclude)docs'], cwd=REPO).stdout
        sh(['git', 'apply', '--whitespace=nowarn', '-'], cwd=ws, input=patch)
    return setup_ref


def checker_issues(ws):
    r = sh([sys.executable, str(CHECKER)], cwd=ws, check=False)
    lines = [l for l in r.stdout.splitlines() if l.strip() and not l.startswith('[')]
    return r.returncode, sorted(set(lines))


# ---------- 运行 agent ----------

def run_agent(case, ws, model, out, target):
    meta = case['meta']
    cmd = ['claude', '-p', case['prompt'], '--output-format', 'stream-json', '--verbose',
           '--permission-mode', 'bypassPermissions', '--no-session-persistence',
           '--max-turns', str(meta.get('max_turns', 60)), '--model', model]
    if meta.get('budget_usd'):
        cmd += ['--max-budget-usd', str(meta['budget_usd'])]
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    env.pop('CLAUDECODE', None)
    t0 = time.time()
    try:
        r = subprocess.run(cmd, cwd=ws, capture_output=True, text=True, env=env,
                           timeout=meta.get('timeout_seconds', 1500))
        raw = r.stdout
    except subprocess.TimeoutExpired as e:
        raw = (e.stdout or b'').decode() if isinstance(e.stdout, bytes) else (e.stdout or '')
    (out / 'trace.jsonl').write_text(raw)
    tools, final, result = [], '', {}
    for line in raw.splitlines():
        try:
            ev = json.loads(line)
        except Exception:
            continue
        if ev.get('type') == 'assistant':
            for b in ev['message'].get('content', []):
                if b.get('type') == 'tool_use':
                    tools.append({'name': b['name'], 'input': b.get('input', {})})
                elif b.get('type') == 'text':
                    final = b['text']
        elif ev.get('type') == 'result':
            result = ev
            final = ev.get('result') or final
    return {'tools': tools, 'final': final, 'cost': result.get('total_cost_usd', 0),
            'turns': result.get('num_turns'), 'seconds': round(time.time() - t0)}


# ---------- grader ----------

def tool_text(t):
    i = t['input']
    return ' '.join(str(i.get(k, '')) for k in ('file_path', 'path', 'command', 'pattern', 'skill', 'prompt'))


SHELL_WRITE = re.compile(r"""sed -i|perl -[a-z]*i|\btee\b|>>?\s*['"]?[\w./-]+\.\w+|write_text|open\([^)]*['"][wa]['"]|git mv|\bmv\b|apply_patch""")


def is_edit(t):
    if t['name'] in ('Edit', 'Write', 'MultiEdit', 'NotebookEdit'):
        return True
    return t['name'] == 'Bash' and bool(SHELL_WRITE.search(t['input'].get('command', '')))


def is_read(t, rx):
    if is_edit(t):
        return False
    return t['name'] in ('Read', 'Bash', 'Grep', 'Glob') and re.search(rx, tool_text(t))


def changed_files(ws, ref):
    out = sh(['git', 'diff', '--name-only', ref], cwd=ws).stdout.split()
    out += sh(['git', 'ls-files', '--others', '--exclude-standard'], cwd=ws).stdout.split()
    return sorted(set(out))


def touched_sections(ws, ref, path):
    p = ws / path
    if not p.exists():
        return set()
    ids, cur = [], '(head)'
    for l in p.read_text().splitlines():
        m = re.search(r'<a id="([^"]+)"></a>', l)
        if m:
            cur = m.group(1)
        ids.append(cur)
    tracked = sh(['git', 'cat-file', '-e', f'{ref}:{path}'], cwd=ws, check=False).returncode == 0
    if not tracked:
        return set(ids) | {'(new)'}
    diff = sh(['git', 'diff', '-U0', ref, '--', path], cwd=ws).stdout
    secs = set()
    for m in re.finditer(r'^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? ', diff, re.M):
        a, n = int(m.group(1)), int(m.group(2) if m.group(2) is not None else 1)
        for i in range(max(a, 1), a + max(n, 1)):
            if i <= len(ids):
                secs.add(ids[i - 1])
    return secs


def judge(criteria, material, model):
    prompt = ('你是严格的评分员。只根据下面的材料判断标准是否满足。第一行只输出 PASS 或 FAIL，第二行给一句理由。\n\n'
              f'## 标准\n{criteria}\n\n## 材料\n{material[:60000]}')
    r = sh(['claude', '-p', prompt, '--model', model, '--no-session-persistence', '--max-turns', '1',
            '--tools', ''], check=False, cwd='/tmp')
    first = (r.stdout.strip().splitlines() or ['FAIL'])[0]
    return first.strip().upper().startswith('PASS'), r.stdout.strip()[:300]


def grade(g, ws, ref, run, base_issues, judge_model):
    t = g['type']
    files = changed_files(ws, ref)
    commits = int(sh(['git', 'rev-list', '--count', f'{ref}..HEAD'], cwd=ws).stdout.strip())
    if t == 'check_library':
        code, issues = checker_issues(ws)
        new = [i for i in issues if i not in base_issues]
        return not new, '; '.join(new[:5]) or 'no new issues'
    if t == 'docs_touched':
        miss = []
        for req in g['require']:
            path, _, secs = req.partition('#')
            if path not in files:
                miss.append(f'{path} unchanged')
            elif secs:
                got = touched_sections(ws, ref, path)
                if not got & set(secs.split('|')):
                    miss.append(f'{path} touched {sorted(got)} not {secs}')
        return not miss, '; '.join(miss) or 'ok'
    if t == 'paths_untouched':
        bad = [f for f in files if any(re.search(rx, f) for rx in g['paths'])]
        return not bad, ', '.join(bad) or 'ok'
    if t == 'no_changes':
        dirty = [f for f in files if not f.startswith(('.claude/', 'target/'))]
        return not dirty and commits == 0, f'files={dirty[:5]} commits={commits}'
    if t == 'file_regex':
        p = ws / g['path']
        text = p.read_text() if p.exists() else ''
        hit = bool(re.search(g['pattern'], text, re.M))
        ok = hit if g.get('match', 'contains') == 'contains' else not hit
        return ok, f"{g['path']} {'has' if hit else 'lacks'} /{g['pattern']}/"
    if t == 'path_exists':
        ok = (ws / g['path']).exists() == g.get('exists', True)
        return ok, g['path']
    if t == 'repo_regex_absent':
        r = sh(['git', 'grep', '-nE', g['pattern'], '--', '.', *[f':(exclude){x}' for x in g.get('exclude', [])]],
               cwd=ws, check=False)
        r2 = sh(f"grep -rnE '{g['pattern']}' $(git ls-files --others --exclude-standard) /dev/null", cwd=ws, check=False)
        hits = (r.stdout + r2.stdout).strip().splitlines()
        return not hits, '; '.join(hits[:3]) or 'ok'
    tools = run['tools']
    if t == 'read_before_edit':
        first_edit = next((i for i, x in enumerate(tools) if is_edit(x) and re.search(g.get('edit', '.'), tool_text(x))), None)
        if first_edit is None:
            return g.get('pass_if_no_edit', False), 'no edit'
        ok = any(is_read(x, g['read']) for x in tools[:first_edit])
        return ok, f'first edit at tool #{first_edit}'
    if t == 'no_read':
        hits = [tool_text(x)[:80] for x in tools if is_read(x, g['read'])]
        skill = [x for x in tools if x['name'] == 'Skill' and re.search(g.get('skill', '^$'), tool_text(x))]
        return not hits and not skill, f'reads={hits[:3]} skill={len(skill)}'
    if t == 'max_reads':
        n = sum(1 for x in tools if is_read(x, g['read']))
        return n <= g['max'], f'{n} reads'
    if t == 'llm':
        if g.get('focus') == 'trace':
            material = '\n'.join(f"{x['name']}: {tool_text(x)[:200]}" for x in tools) + '\n\n最终回复：\n' + run['final']
        elif g.get('focus') == 'diff':
            material = sh(['git', 'diff', ref, '--', 'docs'], cwd=ws).stdout + '\n\n最终回复：\n' + run['final']
        else:
            material = run['final']
        return judge(g['body'], material, judge_model)
    raise ValueError(t)


def one(case, arm, i, args):
    out = Path(args.out) / arm['name'] / case['name'] / f'run{i}'
    out.mkdir(parents=True, exist_ok=True)
    prev = out / 'result.json'
    if prev.exists():
        old = json.loads(prev.read_text())
        if not old.get('invalid') and not old.get('error'):
            return old
    ws = Path(args.work) / f"{arm['name']}-{case['name']}-{i}"
    shutil.rmtree(ws, ignore_errors=True)
    try:
        ref = prepare(case, arm, ws)
        _, base_issues = checker_issues(ws)
        run = run_agent(case, ws, args.model, out, Path(args.work) / 'cargo-target')
        if LIMIT.search(run['final']) or not run['cost']:
            raise RuntimeError('invalid run (usage limit or no output): ' + run['final'][:200])
        grades = []
        for g in case['graders']:
            try:
                ok, why = grade(g, ws, ref, run, base_issues, args.judge_model)
            except Exception as e:
                ok, why = False, f'grader error: {e}'
            if LIMIT.search(why):
                raise RuntimeError('invalid judge (usage limit): ' + why[:200])
            grades.append({'grader': g['name'], 'pass': bool(ok), 'why': why})
        (out / 'diff.patch').write_text(sh(['git', 'diff', ref], cwd=ws).stdout)
        res = {'case': case['name'], 'arm': arm['name'], 'run': i, 'grades': grades,
               'score': sum(g['pass'] for g in grades) / len(grades), 'pass': all(g['pass'] for g in grades),
               'cost': run['cost'], 'turns': run['turns'], 'seconds': run['seconds'],
               'tools': len(run['tools']), 'final': run['final'][-1500:]}
    except Exception as e:
        res = {'case': case['name'], 'arm': arm['name'], 'run': i, 'error': str(e)[-2000:], 'grades': [], 'invalid': True,
               'score': 0, 'pass': False, 'cost': 0}
    (out / 'result.json').write_text(json.dumps(res, ensure_ascii=False, indent=1))
    if not args.keep:
        shutil.rmtree(ws, ignore_errors=True)
    print(f"[{arm['name']}] {case['name']} run{i}: score={res['score']:.2f} pass={res['pass']} cost={res.get('cost', 0):.2f}", flush=True)
    return res


def summarize(results, arms, cases, runs):
    lines = ['| 用例 | ' + ' | '.join(f'{a} 通过/得分' for a in arms) + ' |', '| --- |' + ' --- |' * len(arms)]
    for c in cases:
        row = [c]
        for a in arms:
            rs = [r for r in results if r['arm'] == a and r['case'] == c]
            row.append(f"{sum(r['pass'] for r in rs)}/{len(rs)} · {statistics.mean(r['score'] for r in rs):.2f}" if rs else '-')
        lines.append('| ' + ' | '.join(row) + ' |')
    lines.append('')
    for a in arms:
        rs = [r for r in results if r['arm'] == a]
        per_run = [statistics.mean(r['pass'] for r in rs if r['run'] == i) for i in range(runs) if any(r['run'] == i for r in rs)]
        per_run_score = [statistics.mean(r['score'] for r in rs if r['run'] == i) for i in range(runs) if any(r['run'] == i for r in rs)]
        lines.append(f"- {a}：用例通过率 {statistics.mean(r['pass'] for r in rs):.1%}（各轮 {', '.join(f'{x:.0%}' for x in per_run)}，"
                     f"标准差 {statistics.pstdev(per_run):.1%}）；grader 平均得分 {statistics.mean(r['score'] for r in rs):.3f}"
                     f"（标准差 {statistics.pstdev(per_run_score):.3f}）；花费 ${sum(r.get('cost') or 0 for r in rs):.2f}")
    return '\n'.join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--arm', action='append', required=True, help='name=none 或 name=<变体目录>')
    ap.add_argument('--runs', type=int, default=3)
    ap.add_argument('--case', default='')
    ap.add_argument('--out', required=True)
    ap.add_argument('--work', required=True)
    ap.add_argument('--model', default='claude-opus-5-5')
    ap.add_argument('--judge-model', default='claude-haiku-5-5')
    ap.add_argument('-j', '--concurrency', type=int, default=4)
    ap.add_argument('--keep', action='store_true')
    args = ap.parse_args()
    arms = [dict(zip(('name', 'spec'), a.split('=', 1))) for a in args.arm]
    cases = load_cases(args.case)
    jobs = [(c, a, i) for i in range(args.runs) for c in cases for a in arms]
    with cf.ThreadPoolExecutor(args.concurrency) as ex:
        results = list(ex.map(lambda j: one(*j, args), jobs))
    bad = [r for r in results if r.get('invalid')]
    if bad:
        print(f'{len(bad)} 次运行无效（用量上限或出错），重新执行同一命令会只补跑这些', flush=True)
    results = [r for r in results if not r.get('invalid')]
    Path(args.out, 'results.json').write_text(json.dumps(results, ensure_ascii=False, indent=1))
    md = summarize(results, [a['name'] for a in arms], [c['name'] for c in cases], args.runs)
    Path(args.out, 'summary.md').write_text(md)
    print(md)


if __name__ == '__main__':
    main()
