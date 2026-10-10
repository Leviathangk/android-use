"""Real-tokenizer (o200k) input/output tokens per task from a BENCH_TRACE file.

Usage: python3 trace_tokens.py TRACE.json FIXED_TOKENS
Input tokens per task = sum(result tokens) + calls * FIXED_TOKENS (the tool definitions are
re-read on every model call). Output tokens = the command text plus a tool-call wrapper.
"""
import json, sys
import tiktoken

enc = tiktoken.get_encoding("o200k_base")
t = lambda s: len(enc.encode(s))
fixed = int(sys.argv[2])
rows = []
for task in json.load(open(sys.argv[1])):
    runs = task["runs"]
    n = len(runs)
    calls = sum(len(r["calls"]) for r in runs) / n
    res = sum(t(c[2]) for r in runs for c in r["calls"]) / n
    out = sum(t(c[1]) + 12 for r in runs for c in r["calls"]) / n
    ok = sum(r["ok"] for r in runs)
    rows.append((task["task"], ok, n, calls, res, res + calls * fixed, out))
    print(f"| {task['task']} | {ok}/{n} | {calls:.1f} | {res:.0f} | {res + calls * fixed:.0f} | {out:.0f} |")
k = len(rows)
print(f"| mean | {sum(r[1] for r in rows)}/{sum(r[2] for r in rows)} | {sum(r[3] for r in rows)/k:.1f} | {sum(r[4] for r in rows)/k:.0f} | {sum(r[5] for r in rows)/k:.0f} | {sum(r[6] for r in rows)/k:.0f} |")
