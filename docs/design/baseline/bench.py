"""Baseline benchmark for android-use: drives `au serve --mcp` like an agent client.

Agent policy ("informed"): each call may only quote labels already shown in an
earlier result, chains with `then` only across labels it has seen, and stops a
scroll loop as soon as the target appears. That makes call counts reproducible
and a lower bound for a real model following the current docs.

Usage: python3 -I bench.py AU_BINARY OUT.json
"""

import json, os, subprocess, sys, time
import tiktoken

ENC = tiktoken.get_encoding("o200k_base")
ADB = os.path.expanduser("~/Library/Android/sdk/platform-tools/adb")
FIXTURE = "AU Benchmark Fixture"
RECOVER = os.environ.get("POLICY", "recover") == "recover"


def tok(s):
    return len(ENC.encode(s))


class Mcp:
    def __init__(self, au):
        self.p = subprocess.Popen([au, "serve", "--mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.id = 0
        self.extra = []
        init = self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "bench", "version": "0"}})
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.p.stdin.flush()
        tools = self.rpc("tools/list", {})["tools"]
        self.fixed = tok(init.get("instructions", "")) + sum(tok(json.dumps(t, separators=(",", ":"))) for t in tools)

    def rpc(self, method, params):
        self.id += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        while True:
            line = self.p.stdout.readline()
            msg = json.loads(line)
            if isinstance(msg, dict) and msg.get("id") == self.id:
                return msg["result"]

    def call(self, tool, command):
        r = self.call_once(tool, command)
        # "recover" policy: like a model following the docs, read again and retry
        # when the action missed because the screen had not caught up yet.
        tries = 0
        while RECOVER and tool == "android.act" and tries < 2 and (
                "is on the current screen. Nothing was run" in r["text"] or "no readable controls" in r["text"]):
            tries += 1
            self.extra.append(self.call_once("android.read", "screen"))
            self.extra.append(r)
            r = self.call_once(tool, command)
        return r

    def call_once(self, tool, command):
        start = time.perf_counter()
        r = self.rpc("tools/call", {"name": tool, "arguments": {"command": command}})
        ms = (time.perf_counter() - start) * 1000
        text = "".join(c.get("text", "") for c in r["content"] if c["type"] == "text")
        images = sum(1 for c in r["content"] if c["type"] == "image")
        return {"tool": tool, "command": command, "ms": round(ms), "text": text, "images": images,
                "is_error": r.get("isError", False), "result_tokens": tok(text),
                # what the agent writes: the command plus the tool-call wrapper
                "output_tokens": tok(command) + tok(json.dumps({"name": tool, "input": {"command": ""}}))}


def adb(*args):
    subprocess.run([ADB, *args], capture_output=True)


def reset(pkg):
    # Clear the task too: Android restores a force-stopped app's last page from recents.
    launch = {"com.android.settings": "com.android.settings/.Settings", "dev.codex.aubench": "dev.codex.aubench/.MainActivity"}[pkg]
    adb("shell", "am", "start", "-W", "-S", "--activity-clear-task", "-n", launch)
    adb("shell", "am", "force-stop", pkg)
    adb("shell", "input", "keyevent", "HOME")
    time.sleep(1)


def scroll_until(m, calls, target, first, more, limit=8):
    r = m.call("android.act", first)
    calls.append(r)
    while target not in r["text"] and len(calls) < limit:
        r = m.call("android.act", more)
        calls.append(r)
    return target in r["text"]


def t_wifi(m, calls):
    reset("com.android.settings")
    for c in ['open app "Settings"', 'tap "Network & internet"', 'tap "Internet"']:
        calls.append(m.call("android.act", c))
    if "Wi-Fi" not in calls[-1]["text"]:
        calls.append(m.call("android.read", "screen"))
    r = m.call("android.act", 'tap "Wi-Fi"')
    calls.append(r)
    return not r["is_error"] and "Done" in r["text"]


def t_about(m, calls):
    reset("com.android.settings")
    found = scroll_until(m, calls, "About emulated device", 'open app "Settings"', "scroll down")
    if not found:
        return False
    r = m.call("android.act", 'tap "About emulated device"')
    calls.append(r)
    return "Android version" in r["text"]


def t_form(m, calls):
    reset("dev.codex.aubench")
    calls.append(m.call("android.act", f'open app "{FIXTURE}"'))
    r = m.call("android.act", 'type "hello" in "Fixture text" then tap "SUBMIT FORM"')
    calls.append(r)
    return "Submitted: hello" in r["text"]


def t_dialog_cancel(m, calls):
    reset("dev.codex.aubench")
    calls.append(m.call("android.act", f'open app "{FIXTURE}"'))
    calls.append(m.call("android.act", 'tap "OPEN BENCHMARK DIALOG"'))
    r = m.call("android.act", 'tap "CANCEL"')
    calls.append(r)
    return "Benchmark confirmation" not in r["text"] and "SUBMIT FORM" in r["text"]


def t_dialog_confirm(m, calls):
    reset("dev.codex.aubench")
    calls.append(m.call("android.act", f'open app "{FIXTURE}"'))
    calls.append(m.call("android.act", 'tap "OPEN BENCHMARK DIALOG"'))
    r = m.call("android.act", 'tap "CONFIRM"')
    calls.append(r)
    return "Dialog confirmed" in r["text"]


def t_toggle(m, calls):
    reset("dev.codex.aubench")
    calls.append(m.call("android.act", f'open app "{FIXTURE}"'))
    r = m.call("android.act", 'tap "Benchmark toggle"')
    calls.append(r)
    return "on switch" in r["text"]


def t_row40(m, calls):
    reset("dev.codex.aubench")
    calls.append(m.call("android.act", f'open app "{FIXTURE}"'))
    if not scroll_until(m, calls, "Fixture row 40", 'scroll down in "Benchmark scroll surface"', 'scroll down in "Benchmark scroll surface"', 10):
        return False
    r = m.call("android.act", 'tap "Fixture row 40"')
    calls.append(r)
    return not r["is_error"]


def t_search(m, calls):
    reset("com.android.settings")
    calls.append(m.call("android.act", 'open app "Settings"'))
    r = m.call("android.act", 'tap "Search Settings"')
    calls.append(r)
    if "Search" not in r["text"]:
        return False
    r = m.call("android.act", 'type "battery" in "Search settings"')
    calls.append(r)
    if "Battery" not in r["text"]:
        r = m.call("android.read", "screen")
        calls.append(r)
    r = m.call("android.act", 'tap "Battery"')
    calls.append(r)
    return not r["is_error"] and "Battery" in r["text"]


def t_check(m, calls):
    calls.append(m.call("android.read", "status"))
    r = m.call("android.read", "screen")
    calls.append(r)
    return not r["is_error"]


TASKS = [("toggle_wifi", t_wifi), ("find_setting_scroll", t_about), ("fill_submit_form", t_form),
         ("dismiss_dialog", t_dialog_cancel), ("confirm_dialog", t_dialog_confirm), ("toggle_switch", t_toggle),
         ("scroll_to_row_40", t_row40), ("settings_search", t_search), ("status_and_screen", t_check)]


def main():
    au, out = sys.argv[1], sys.argv[2]
    runs = int(os.environ.get("RUNS", "3"))
    m = Mcp(au)
    results = []
    for name, fn in TASKS:
        for run in range(runs):
            calls = []
            m.extra = []
            t0 = time.perf_counter()
            try:
                ok = fn(m, calls)
            except Exception as e:  # a crashed task counts as a failure
                ok = False
                calls.append({"error": repr(e)})
            wall = (time.perf_counter() - t0) * 1000
            good = [c for c in calls + m.extra if "ms" in c]
            res = sum(c["result_tokens"] for c in good)
            results.append({"task": name, "run": run, "ok": ok, "calls": len(good), "wall_ms": round(wall),
                            "result_tokens": res, "fixed_tokens": m.fixed * len(good),
                            "input_tokens": res + m.fixed * len(good),
                            "output_tokens": sum(c["output_tokens"] for c in good), "trace": calls})
            print(f"{name} run{run}: ok={ok} calls={len(good)} wall={wall:.0f}ms in={res + m.fixed * len(good)} out={sum(c['output_tokens'] for c in good)}", flush=True)
    json.dump({"fixed_per_turn": m.fixed, "results": results}, open(out, "w"), indent=1, ensure_ascii=False)


if __name__ == "__main__":
    main()
