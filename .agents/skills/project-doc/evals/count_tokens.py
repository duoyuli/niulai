#!/usr/bin/env python3
"""统计常驻文本的 token 数：同一模型下「附带文本」与「不带文本」两次请求的输入 token 之差。

用法：python3 count_tokens.py [--model M] 文件[#gate] ...
文件后加 #gate 表示只取「## Project Doc gate」到文件末尾。
"""
import json, subprocess, sys, tempfile

GATE_HEAD = '## Project Doc gate'


def usage(prompt, model, cwd):
    r = subprocess.run(['claude', '-p', prompt, '--model', model, '--output-format', 'json', '--tools', '',
                        '--no-session-persistence', '--max-turns', '1'], capture_output=True, text=True, cwd=cwd)
    u = json.loads(r.stdout)['usage']
    return u['input_tokens'] + u.get('cache_creation_input_tokens', 0) + u.get('cache_read_input_tokens', 0)


def main():
    args = sys.argv[1:]
    model = 'claude-opus-5-5'
    if args[:1] == ['--model']:
        model, args = args[1], args[2:]
    lead = '只回复 OK。'
    with tempfile.TemporaryDirectory() as cwd:
        base = usage(lead, model, cwd)
        total = 0
        for spec in args:
            path, _, part = spec.partition('#')
            text = open(path).read()
            if part == 'gate':
                text = GATE_HEAD + text.split(GATE_HEAD, 1)[1]
            n = usage(lead + '\n\n' + text, model, cwd) - base
            total += n
            print(f'{n:6d}  {spec}')
        print(f'{total:6d}  合计')


if __name__ == '__main__':
    main()
