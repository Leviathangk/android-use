"""Prototype denser screen formats on recorded screens and count tokens.

Rows are parsed back from the current `screen` rendering (label — parts kind),
which carries the same fields as the helper's semantic rows.
"""

import pathlib, re, sys
import tiktoken

ENC = tiktoken.get_encoding("o200k_base")
KINDS = {"button": "btn", "switch": "sw", "text field": "edit", "scroll area": "list", "checkbox": "chk",
         "option": "radio", "tab": "tab", "link": "link", "slider": "slider"}
STATES = {"on", "off", "checked", "unchecked", "selected", "disabled", "empty"}


def tok(s):
    return len(ENC.encode(s))


def parse(text):
    rows = []
    for line in text.strip().splitlines():
        label, _, rest = line.partition(" — ")
        number = None
        m = re.search(r" \(number (\d+)\)$", label)
        if m:
            label, number = label[: m.start()], int(m.group(1))
        kind, value, state = "", rest, ""
        for long in sorted(KINDS, key=len, reverse=True):
            if rest == long or rest.endswith(" " + long):
                kind, value = long, rest[: -len(long)].strip()
                break
        words = value.split(" ")
        if kind and words and words[-1] in STATES:
            state, value = words[-1], " ".join(words[:-1])
        if kind == "text field" and value == label:
            value = ""  # the hint repeats the label
        rows.append({"label": label, "kind": kind, "value": value, "state": state, "number": number})
    return rows


def v1(rows, summaries=True, handles=False, cut=None):
    """kind prefix, `=value` for state/values, `#N` only on duplicates."""
    out = []
    for i, r in enumerate(rows, 1):
        head = (f"e{i} " if handles else "") + (KINDS[r["kind"]] + " " if r["kind"] else "")
        line = head + r["label"] + (f"#{r['number']}" if r["number"] else "")
        if r["kind"] == "switch":
            line += "=" + (r["state"] or "off")
        elif r["kind"] == "text field":
            line += f'="{r["value"]}"'
        elif r["kind"] == "checkbox":
            line += "=" + ("on" if r["state"] == "checked" else "off")
        elif r["value"] and summaries:
            v = r["value"]
            if cut and len(v) > cut:
                v = v[:cut].rstrip(", ") + "…"
            line += ": " + v
        out.append(line)
    return "\n".join(out)


def diff(before, after):
    """Changed rows only, same screen identity (first row is the title)."""
    b, a = parse(before), parse(after)
    if not b or not a or b[0]["label"] != a[0]["label"]:
        return None
    bs = {r["label"]: r for r in b}
    out = []
    for r in a:
        if r["label"] not in bs:
            out.append("+ " + v1([r]))
        elif bs[r["label"]] != r:
            out.append("~ " + v1([r]))
    for r in b:
        if r["label"] not in {x["label"] for x in a}:
            out.append("- " + r["label"])
    return "\n".join(out)


def main(folder):
    folder = pathlib.Path(folder)
    print("| screen | current | V1 kind-prefix | V2 +summaries cut at 24 | V3 V2 + eN handles | V4 no summaries |")
    print("|---|---|---|---|---|---|")
    tot = [0] * 5
    for f in sorted(folder.glob("*.txt")):
        if f.name.count(".") > 1:  # skip .full.txt and generated .vN.txt
            continue
        text = f.read_text()
        rows = parse(text)
        vs = [text.strip(), v1(rows), v1(rows, cut=24), v1(rows, cut=24, handles=True), v1(rows, summaries=False)]
        ts = [tok(v) for v in vs]
        tot = [a + b for a, b in zip(tot, ts)]
        print(f"| {f.stem} | " + " | ".join(f"{t} ({100 * (t - ts[0]) // ts[0]:+d}%)" if i else str(t) for i, t in enumerate(ts)) + " |")
        (folder / f"{f.stem}.v2.txt").write_text(vs[2] + "\n")
        (folder / f"{f.stem}.v3.txt").write_text(vs[3] + "\n")
    print("| **total** | " + " | ".join(f"**{t}** ({100 * (t - tot[0]) // tot[0]:+d}%)" if i else f"**{t}**" for i, t in enumerate(tot)) + " |")


if __name__ == "__main__":
    main(sys.argv[1])
