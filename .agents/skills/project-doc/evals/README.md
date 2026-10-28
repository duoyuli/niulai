# project-doc 评测

用例格式沿用 `architecture-review/evals/`：每个目录一个用例，含 `case.yaml`、`prompt.md`、`graders/*.md`，可选 `setup.sh`。`claude plugin eval` 没有脚本 grader，也不能在指定提交上回放并检查 `git diff`，所以用 `run.py` 运行。

## 运行

```bash
# 在仓库根目录；变体目录含 skill/（不含 evals/）和 gate.md（AGENTS.md 中 gate 到文件末尾）
uv run --with pyyaml==6.0.2 python .agents/skills/project-doc/evals/run.py \
  --arm baseline=none --arm current=<变体目录> --runs 3 --out <结果目录> --work <临时目录> -j 6
python3 .agents/skills/project-doc/evals/count_tokens.py AGENTS.md#gate docs/index.md <变体目录>/skill/SKILL.md
```

`baseline=none` 删除 skill 和 gate，保留 AGENTS.md 其余内容。每次运行都在独立的 `git clone --shared` 中进行，删除 origin，不会推送到原仓库。

## case.yaml 的 source

- `kind: replay`：检出 `base`（真实提交的父提交）；有 `apply` 时把该提交 `docs/` 以外的改动放进工作区，作为未提交改动；`gold` 记录标准答案所在的提交。
- `kind: head`：检出 `base`，可用 `setup.sh` 构造起点，setup 的改动会先提交。

## grader 类型

| type | 判定 |
| --- | --- |
| `check_library` | `scripts/check_library.py` 不出现 setup 之后新增的问题 |
| `docs_touched` | `require` 中每个 `路径#章节ID|章节ID` 都被改到，章节按改动行之前最近的 `<a id>` 计 |
| `paths_untouched` | 改动文件都不匹配 `paths` 中的正则 |
| `no_changes` | 没有改动、没有新提交 |
| `file_regex` | 文件内容匹配（`match: contains` 或 `not_contains`） |
| `path_exists` | 路径存在与否等于 `exists` |
| `repo_regex_absent` | 已跟踪和新增文件中都没有匹配，可用 `exclude` 排除目录 |
| `read_before_edit` | 第一次编辑匹配 `edit` 的文件前，读过匹配 `read` 的文件 |
| `no_read` | 没有读匹配 `read` 的文件，没有调用名称匹配 `skill` 的 skill |
| `max_reads` | 读匹配 `read` 的文件次数不超过 `max` |
| `llm` | 用 Haiku 按正文判定；`focus` 为 `last_message`、`trace` 或 `diff` |

用例通过要求全部 grader 通过；得分是通过的 grader 占比。
