# Android Use: from-scratch redesign proposal

Status: proposal. Nothing in this document is implemented yet.

This proposal comes from a full read of the repository at `5509535`: the Rust host (`computer/`, about 5.3k lines), the Android helper (`device/`, about 1.6k lines), `tools/` (about 3.1k lines), the docs, and CI. Every claim marked as a defect below was checked against the code. Latency figures are estimates derived from the code paths, not measurements.

## 1. What the current design gets right (keep it)

- **Two MCP tools that take one command string.** `android.read` and `android.act` give a small, cache-friendly tool surface, and the quoted-label grammar is easy for a model to copy.
- **Semantic-first observation.** Accessibility labels come first and screenshots are the fallback. Compact screens are budgeted to about 120 tokens (`format.rs:214`).
- **Explicit outcome vocabulary.** `stale`, `partial` and `unknown` are named, and AGENTS.md states the correct retry rules.
- **A small dependency set.** Seven runtime crates; no async runtime; no shell interpolation (`device.rs:231`).
- **Uid-gated abstract sockets.** Only peer uid 2000 is served (`Bridge.java:38`), plus a release-signer pin in CI.

## 2. What limits it today

### 2.1 Speed: too many round trips and process spawns

| Path | Today | Where |
| --- | --- | --- |
| Cold CLI call | About 7 `adb` processes: devices, getprop, am start, forward ×2, remove ×2 | `device.rs:132-213`, `bridge.rs:37-75` |
| Each `adb` call | 2 reader threads plus a 10 ms `try_wait` poll | `device.rs:248-261` |
| One `tap "X"` | observe RT, then resolve RT, then act RT; no settle; the agent then usually issues another read | `engine.rs:488, 556, 741` |
| Screenshot | Lossless PNG q100 with fsync, then pulled in **2,800-byte chunks**: about 375 sequential RTs per MiB | `Capture.java:53`, `api.rs:5`, `engine.rs:720-738` |
| Browser read/act | New HTTP `GET /json` each call, a fingerprint eval per act, 50 ms polling for tabs and waits | `browser.rs:79, 110, 163, 219, 246-273` |
| Journal | Two `sync_all` per mutation; O(n²) load; full rewrite at the 1024-entry cap | `engine.rs:98-165` |
| Device snapshot | Whole tree walk under a lock that blocks events; semantics recompiled on every read; `typeAllMask` plus not-important views | `Ui.java:42`, `accessibility_service.xml` |

### 2.2 Accuracy: the agent is told things that are not true

The following defects were verified in code:

1. **Unknown outcomes become protocol errors.** The synthetic `{"ok":0,"e":"unknown"}` (`engine.rs:751, 754, 777, 792, 810`) lacks the required `g` and `m` fields of `Receipt` (`api.rs:833-838`). Deserialization in `model_receipt` fails, and the agent gets "Android returned an invalid receipt" instead of the read-and-reconcile guidance.
2. **Chains keep going after failure.** In `model_act` (`engine.rs:390-401`), a `stale`, `partial` or `unknown` receipt is `Ok(text)`, so the next group still runs. For example, `tap "A" then page click "B"` runs B after A's outcome is unknown. This contradicts AGENTS.md.
3. **Every non-host error is reported as success.** `isError:false` is hard-coded (`adapter.rs:102`).
4. **Silent substring targeting.** When no exact label match exists, any unique *containing* label is acted on (`engine.rs:536`), and the receipt echoes the requested label. So `tap "Save"` taps "Save draft" and reports "Tapped Save".
5. **Only the active window is read.** `getRootInActiveWindow` is used (`Ui.java:127`, `Vm.java`) despite `flagRetrieveInteractiveWindows`, so dialogs, IME, system UI and split-screen content are missing or wrongly merged.
6. **No effect verification.** "Done" means the device accepted the dispatch. Gestures are fired with a null callback (`Vm.java: dispatchGesture(g,null,null)`), and there is no settle wait.
7. **Silent truncation.** Device-side observe and semantic caps return no truncated flag (`Ui.java:43, 55`). The host appends "N more omitted" *before* byte truncation, so that notice is the first thing cut (`format.rs:209-216`). In full mode, the tail hint tells the agent to "Read screen full" again, which loops (`format.rs:215`).
8. **Wrong capability report.** `microphone` is `AUDIO_SERVICE != null`, which is always true (`Capture.java`).
9. **Mis-typed JSON-RPC errors.** Every JSON-RPC error, including unknown methods and `ping`, is returned as `-32602` (`adapter.rs:63`).
10. **The fuzz test exercises almost nothing.** It feeds random bytes to `serde_json` (`tests/protocol_fuzz.rs`), so the plan parsers are almost never reached, and `parse_act_command`, the grammar agents actually use, is not fuzzed.
11. **The documented grammar disagrees with the parser.** Examples: `protocol.md` says `camera F [W by H]` while the parser wants `take rear camera photo at W by H`. It also documents `capture` for what is really `capture screen`, and `notification open|dismiss|action` for the real forms. Docs say links are HTTP(S)-only, but `geo:` and `google.navigation:` are accepted (`api.rs:756`).

The reviewers also reported the following, but I did not independently re-verify them:

- Pre-send failures leave `Pending` journal entries that are never evicted (`engine.rs:117, 746-752`).
- The browser label fallback can surface typed `value`s (`browser.rs:365`).
- Idle helper sockets time out after 35 s, so the next act returns `unknown`.
- Batches check the generation only before the first mutation (`Vm.java:30`).

### 2.3 Agent-nativeness: prose-only, read-act-read loop

- Results are prose only, with no machine-readable status, resolved target, generation or change flag.
- Every action needs a follow-up read to learn what happened. A three-step task costs about six tool calls.
- Targets are label strings plus implicit ordinals, and `screen` and the resolver count ordinals differently (`format.rs:183` vs `engine.rs:535`).
- There are about 45 verb forms with synonyms (`open notification` / `run notification action`; `wait for "T"` / `wait for text`). There is no `scroll until` and no paging.
- Grammar is restated in about 7 files and retry rules in about 8, and they have already drifted.

### 2.4 Code: dense, not small

The code is already "minified" by formatting rather than by design:

- 49 lines exceed 300 characters, and one line in `Capture.java` is about 3.6k characters.
- The cause is `xtask gate`, which budgets **line counts** (`tools/src/main.rs:562`: rust 6500 / java 2200).

Real redundancy remains:

- **Two grammars and four action enums.** `AndroidAction`/`BrowserAction` live in `command.rs` and `Op`/`BrowserOp` in `api.rs`, with duplicated URL, key and bounds validators.
- **Three copies of the chunked-fetch loop** and three of forward teardown.
- **A legacy JSON protocol** and a legacy scene formatter, plus a `--jsonl` shim.
- **Dead code:** `visual::Op`, `api::Req`, `Engine::root`, the modal path on the device, and `SDK_INT<21` guards with `minSdk` 26.
- **A 2.2k-line TypeScript lint plugin** that polices about 94 lines of JS.

## 3. The redesign

### 3.1 Principles

1. **One round trip per intent.** The device resolves the target, acts, waits for the UI to settle, and returns a diff, all in a single frame.
2. **Every act returns what changed.** The agent never needs a read just to learn the outcome.
3. **Handles over guesses.** Observations assign short element handles (`e12`) bound to a generation, and labels are a convenience layer over them.
4. **Structured first, prose second.** MCP `structuredContent` carries the truth, and the text block is a rendering of it.
5. **One spec, generated everywhere.** The grammar table drives the parser, the tool description, the docs and the tests.
6. **Small by deletion, readable by default.** Budget artifact bytes and concepts, not lines.

### 3.2 Architecture

```text
agent ──MCP stdio──▶ au (host, Rust, ~2.5k lines)
                      │  grammar ─▶ intent IR ─▶ link ─────────────┐
                      │  render (text + structuredContent)          │ one persistent stream
                      │  journal (state machine, append-only)       │ via adb server protocol
                      ▼                                             ▼ (no subprocess, no forward)
                 unix socket ◀─ `au` CLI calls          aubridge (Android, ~1.2k lines)
                 (daemon reuse)                           AccessibilityService
                                                          ├─ WindowModel (getWindows, event-coalesced cache)
                                                          ├─ Resolver (exact → normalized → candidates)
                                                          ├─ Executor (action + settle + diff)
                                                          └─ Capture (JPEG/WebP, streamed)
```

#### Transport

- **Talk to the adb server directly over TCP 5037.** Send `host:transport:<serial>`, then `localabstract:aubridge`. This is the same smart-socket protocol `adb` itself uses, at about 120 lines of Rust. It removes every subprocess spawn from the hot path, and `adb forward` with it. The `adb` binary is still used for `install` and setup only.
- **Binary frames.** Each frame is `u32 len | u8 kind | payload`. `kind` is `json` for control or `blob` for image and media bytes. A screenshot streams as one blob instead of about 375 base64 chunk requests.
- **Run as a daemon.** `au serve` owns the connection. CLI invocations reuse it through a unix socket or named pipe, so cold-connect cost is paid once per session, not once per command.
- **Keepalive, not a 35 s idle death.** The helper pings, and the host reconnects transparently *before* sending. A reconnect therefore never turns into `unknown`.

#### Auth

Keep the uid-2000 peer check; it is meaningful because the peer is now always `adbd`.

- **Never send the secret over the wire.** At setup, the host generates a 32-byte secret and writes it into the helper through an `adb shell content call` restricted to the shell uid. Each connection then does an HMAC challenge-response over a helper nonce. This removes the bootstrap socket that hands token and nonce to any local caller.
- **Treat CDP the same way.** Chrome DevTools is reached through the same adb stream (`localabstract:chrome_devtools_remote`), so there is no unauthenticated loopback TCP port on the host.

#### Device model (the main accuracy and speed lever)

- **Multi-window.** Use `getWindows()`, ordered by layer. A modal dialog or IME *dominates*: background windows are reported as `blocked` and are not targetable.
- **Event-coalesced cache.** Keep a dirty flag plus a 40–80 ms trailing debounce. Walk the tree off the event thread with no global lock held. Compile semantics once per generation and cache them.
- **Stable keys.** Each node gets a key of `hash(window, viewIdResourceName, class, ancestor-path, label)`. A handle `e12` maps to a key, not a pointer, so a handle survives re-layouts that keep the same element. When it does not survive, the outcome is `stale`, with the new candidates attached.
- **Narrower event mask.** Subscribe to `WINDOW_STATE_CHANGED`, `WINDOW_CONTENT_CHANGED`, `WINDOWS_CHANGED`, `VIEW_SCROLLED`, `VIEW_TEXT_CHANGED` and `VIEW_FOCUSED` instead of `typeAllMask`. Drop `flagIncludeNotImportantViews` except on an explicit `screen full`.
- **Richer rows.** Report kind, label, value, state (checked/selected/disabled/focused), scrollable direction with `CollectionInfo` counts ("list, 3 of 48 shown, more ↓"), and redaction for password and sensitive fields.

#### Executor: act, then settle, then diff

```text
act(intent, generation):
  resolve target on-device (handle → key → node; label → exact/normalized; >1 → ambiguous + candidates)
  if generation moved and target key changed → stale (+ fresh candidates)      # never act blind
  perform (performAction; dispatchGesture WITH completion callback)
  settle: wait until no content events for 120 ms, or 1.5 s cap (configurable per intent)
  diff: added / removed / changed rows since pre-act generation, window change, toast text
  return {status, resolved:{handle,label}, changed:bool, diff, g}
```

- **Batches re-validate before every mutation**, not just the first.
- **A batch stops at the first non-`done` step**, and the outcome reports which step it was.
- **Idempotency is device-side.** Keep a `(session, request_id)` ring held under one lock, which closes today's window between `running.remove` and `done.put`.

#### Host

| Module | Responsibility | Target lines |
| --- | --- | --- |
| `spec.rs` | One table: verb → args → bounds → IR; also emits the tool description and the doc reference | 300 |
| `grammar.rs` | Lexer plus table-driven parser | 250 |
| `link.rs` | adb smart-socket client, frames, HMAC, reconnect | 350 |
| `engine.rs` | Intent dispatch and the journal state machine (`queued → sent → done/unknown`, ordered map, rotation instead of a 2 MiB refusal) | 500 |
| `render.rs` | structuredContent plus token-budgeted text; Unicode-safe truncation; counts placed before truncation | 350 |
| `web.rs` | One CDP session: target events instead of `/json` polling, `Accessibility.getFullAXTree` for page semantics, an overall deadline | 450 |
| `mcp.rs` / `main.rs` | JSON-RPC (proper `-32601`, `ping`), daemon, CLI | 300 |
| **Total** | | **~2.5k** (from ~5.3k) |

Delete:

- The legacy JSON plan protocol and `--jsonl`, after one deprecation release.
- `visual.rs` and the `png` and `sha1` dependencies. Image hashing and diffing move to the helper on thumbnails, or are dropped.
- The legacy scene formatter, the guidance-JSON builders in `install.rs` (move them to static data), and the duplicated validators.

### 3.3 Agent protocol v2

Keep exactly two tools. Each takes one `command` string plus an optional `request_id`.

#### Observation

The observation starts with a header line, followed by one row per element:

```text
g57 Settings › Network & internet           [window: com.android.settings]
e1 button "Back"
e2 heading "Network & internet"
e3 switch "Wi-Fi" on — "HomeNet"
e4 item "Mobile network"
e5 item "Airplane mode" off
e6 list 5 of 9 shown, more ↓
```

#### Act result

Each act returns its outcome and the screen diff, so no follow-up read is needed:

```text
done: tapped e3 "Wi-Fi" → changed
~ e3 switch "Wi-Fi" off
- "HomeNet"
```

The matching `structuredContent`:

```json
{"status":"done","resolved":{"h":"e3","label":"Wi-Fi"},"changed":true,"g":58,
 "diff":{"changed":[["e3","switch","Wi-Fi","off"]],"removed":[["text","HomeNet"]]}}
```

#### Target rules

1. **Handles are exact.** `e3` is checked against the generation. If the key is unchanged the act proceeds; if it moved, the outcome is `stale` with candidates.
2. **Quoted labels match exactly after normalization** (case, whitespace, punctuation). Two or more matches give `ambiguous` with the candidate handles listed. Zero matches give `not_found` with the three nearest labels as *suggestions*. Nothing is ever acted on through a fuzzy match.
3. **`tap point X Y` stays a bounded fallback**, valid only for the generation it was issued against.

#### Outcomes

`done | no_change | stale | ambiguous | not_found | blocked | denied | partial(step) | unknown`.

- Everything except `done` and `no_change` sets `isError:true`.
- Chains stop at the first non-`done` step.
- The retry rule is mechanical and stated once: `stale`, `ambiguous` and `not_found` are pre-send, so act again with the new info. `partial` and `unknown` mean "read before mutating".

#### Verbs (about 18, no synonyms)

- Read: `screen [full|more]`, `find "T"`, `status`, `page [text]`, `tabs`.
- Act: `tap T`, `hold T`, `type "X" [in T]`, `clear T`, `scroll D [in T] [until "X"]`, `swipe …`, `press KEY`, `open app|link|setting "V"`, `wait for "X" [N s]`, `back`, `go "URL"`, `tab new|select|close`.

Web pages share the handle namespace: CDP AX-tree nodes appear as handles (`w4 link "Visitor centers"`). `tap w4` therefore works without a separate `page click` vocabulary. CDP is used only where accessibility is weaker, such as page text, URL, navigation and in-page waits.

#### Visual fallback with marks

`screen image` returns a downscaled (1024 px long edge) JPEG with handle labels drawn on the elements, in the style of set-of-marks prompting. The visual path then still produces `tap e12` rather than raw coordinates.

#### Sensitive capabilities

Camera, microphone, screen recording, location and notifications move behind a host-side grant (`au serve --allow camera,location`). Without the grant, these verbs return `denied`. Today the only control is AGENTS.md prose.

### 3.4 Docs, skill and tooling

- **Generate, don't restate.** `cargo xtask spec` emits `skills/android-use/references/protocol.md`, the MCP tool description, and the grammar test corpus from `spec.rs`. CI fails on any diff. This removes the whole drift class in §2.2 item 11.
- **Three docs where there are now 27 files.** A human README, AGENTS.md (≤1.5 KB, the only agent-loaded file), and the generated reference. Fold setup steps into a single page.
- **Budget what matters.** Gate on release artifact bytes, skill token count and cyclomatic complexity, not line counts. Format with stock `rustfmt` and `google-java-format`. Minification belongs in the build: R8 is already on, and `opt-level="z"` + fat LTO is already set for `dist`.
- **CI.** Cache cargo and gradle, run on `pull_request` plus `push` to `main` only, and use prebuilt `cargo-audit` and `cargo-deny`. Run the docs, grammar and golden gates on every PR. Add a nightly emulator job that drives the *shipped* grammar; today `live` and `benchmark` drive the deprecated JSON path (`tools/src/main.rs:205-216`).
- **Tests.** Add a fake adb server and fake helper speaking the frame protocol, so handshake, reconnect, seq mismatch and receipt mapping get covered. Fuzz `grammar::parse` and the frame decoder with `cargo-fuzz`. Add an emulator test for settle and diff.
- **Delete the oxlint anti-slop plugin**, or keep two rules. Its attribution is also missing from THIRD_PARTY_NOTICES.
- **Rename before the identity sets.** The `applicationId` is `dev.codex.aubridge`, and setup prints internals ("UI generation 44", "Capability mask").

## 4. Expected effect

These are estimates from the code paths above. Validate them with the benchmark harness before claiming them.

| Metric | Today | Redesign |
| --- | --- | --- |
| Tool calls for "tap X, confirm result" | 2–3 (act, then a read) | 1 |
| Device round trips per tap | 3 (observe, resolve, act) | 1 (includes settle and diff) |
| Process spawns per CLI call | ~7 | 0 (daemon) |
| Screenshot transfer, 1 MiB | ~375 sequential RTs, lossless PNG | 1 streamed blob, ~150 KB JPEG |
| Wrong-target risk | Silent substring match; wrong-window content | Exact or handle only; dominated windows excluded |
| Host + device code | ~6.9k lines (dense) | ~3.7k lines (normally formatted) |
| Agent-facing docs | ~1.05k tokens loaded, 27 files drifting | ≤600 tokens loaded, 1 generated reference |

## 5. Recommended path: staged, not big-bang

A rewrite is not needed to capture most of this. Each stage below ships on its own.

1. **Stop lying to the agent** (days). Fix §2.2 items 1–4 and 7–10: a complete synthetic unknown receipt, stop-on-first-non-ok, `isError`, removal of the substring fallback (or making it return `ambiguous`), truncation counts placed first, `-32601`/`ping`, the mic capability, and a fuzzer on the real grammar. Regenerate protocol.md from the parser.
2. **One round trip plus diff** (largest win). Make on-device resolve, settle with the gesture callback, and diff the receipt. Add `structuredContent`.
3. **Transport.** Add the adb smart-socket client, binary blob frames, daemon reuse and HMAC auth.
4. **Window model and handles.** Add `getWindows()`, stable keys, handles and `scroll until`.
5. **Unify and delete.** Add `spec.rs` generation, remove the legacy JSON protocol and `visual.rs`, merge web into the handle namespace, and cut docs to three.
