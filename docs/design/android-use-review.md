# android-use — code review and improvement report

Reviewed: `austinintelligence/android-use` at `5509535` (all Rust, Java, installer, CI, and docs).
Changes are on the local branch `improve/agent-native-reliability` (uncommitted).

## Overall assessment

The foundation is strong. The safety model is genuinely good:

- generation-checked plans
- an at-most-once journal
- an authenticated helper with no `INTERNET` permission
- bounded everything
- no shell or JavaScript escape hatch

The two-tool, plain-command interface is the right shape for agents.

The weaknesses were in the last mile, where the agent meets the device. Several agent-facing features silently did not work, the server could fail to start in ways agents can't see, transfers were very chatty, and much of the Java was written as dense one-line methods that are hard to review. The CHANGELOG also overstated some things. "Open, dismiss, or invoke … notification", for example, could never succeed through the agent interface.

## Bugs found (all fixed)

| # | Severity | Where | Problem |
| --- | --- | --- | --- |
| 1 | High | `format.rs` `format_notifications` | Helper returns `{enabled, items:[…]}`; host called `.as_array()` on it → **always** "no actionable notifications". |
| 2 | High | `engine.rs` notification ops | `open notification "Messages"` sent the *label* where the helper looks up Android's internal notification key → could never match. |
| 3 | High | `format.rs` `format_summary` | `location` / `capabilities` returned only key names ("available for lat, lon") — never the values. |
| 4 | High | `adapter.rs` + journal | Operation ids = hash(JSON-RPC id). Ids restart per client session; the journal is durable → after a client restart, actions fail with "operation id was already used" (or, on digest match, replay an old receipt). |
| 5 | High | `main.rs` `serve` | `Engine::open()` resolved the device before serving → no device/adb = MCP server exits; the agent just sees "server failed". |
| 6 | Medium | `engine.rs` `model_act` | A semantic miss in step 1 returned `Ok` and **later `then` steps kept running**. Failed receipts (stale, failed verify) also continued to the next group. |
| 7 | Medium | `status` | Reported "ready" while the Accessibility service was off (helper answers `cap=0`). |
| 8 | Medium | `Ui.resolve` | Matched only text/description, so a filled field shown as "Name — Ada" couldn't be targeted by `type … in "Name"`. |
| 9 | Medium | `adapter.rs` | Audio/MP4 were sent as MCP `image` content blocks; unknown methods returned -32602 instead of -32601; `ping` errored. |
| 10 | Medium | `engine.rs` | Pre-send connection failures were reported as `unknown` and left pending journal records. |
| 11 | Low | `command.rs` | `page scroll` unbounded on the command path (legacy path enforced ±20000). |
| 12 | Low | `engine.rs` `store_image` | Every capture added a permanent `image N` alias → unbounded memory growth in long sessions. |
| 13 | Low | `browser.rs` | Page screenshots always labelled `image/jpeg`, even when the PNG helper fallback was used. |
| 14 | Low | `Bridge.apps` | `loadLabel()` called twice per app on every `open app` lookup. |
| 15 | Low | baseline | `clippy::collapsible_match` failure under Rust 1.95 (CI pins 1.94, so it was latent). |

## Speed

| Change | Effect |
| --- | --- |
| 256 KiB artifact slices (was 2,800 B), auto-fallback for old helpers | 1080×2400 screenshot: **573 → 8** round trips; 1440×3120: **1251 → 15**. |
| Post-action screen summary | Removes the read turn an agent otherwise needs after nearly every action — the biggest end-to-end win, since each agent turn costs seconds. |
| Default `screen` budget 480 → 1,200 B | Typical screens fit in one read instead of `screen` + `screen full`. |
| Fast reconnect | Skips `adb shell am start-foreground-service` when the helper is already up. |
| Journal finish without `F_FULLFSYNC` | One fewer full-disk flush per action (pending record still durable). |
| App label cache (30 s) | `open app "X"` no longer reloads every app's label resources. |

## Agent-native improvements

- **Results carry state.** `Done. Tapped Wi-Fi.` + `Now on screen:` (after a bounded settle).
- **Chains cross screens.** `open app "Settings" then tap "Wi-Fi"` used to fail outright, because every target was resolved against the starting screen. Now the prefix runs as its own generation-checked plan, the UI settles, and the rest resolves on the new screen.
- **Every failure says what ran and what to do**: step name, whether anything changed, one next move.
- **Self-describing tools**: tool descriptions contain the whole grammar; server `instructions` on `initialize`.
- **Duplicates show the exact disambiguator**: `Save (number 2) — button`.
- **`scroll down`** with no label (helper picks the largest area that advertises that direction).
- **`au read` / `au act`** — same grammar for shell-only agents.
- **Screenshots compressed for vision models** (`compress.rs`): content classification, area-averaged resize, exact-or-median-cut palette PNG, byte budget, and exact coordinate mapping for `tap point` with corrected context for out-of-range points and rotation.
- **Server starts without a device**; errors become actionable tool results.

## Architecture changes

- `engine.rs` (867 lines, at the 900-line gate) → `engine.rs` (core), `agent.rs` (agent layer), `journal.rs`, `resolve.rs`, standalone `format.rs`; `api.rs` legacy parsing → `legacy.rs`.
- One resolver rule for screens, pages, tabs, apps, notifications (previously five near-duplicates with subtly different rules).
- `Error::plain` distinguishes agent-ready messages from internal diagnostics (replaces the never-used `trace` field); new `Code::Missing`.
- Helper Java reformatted with google-java-format (AOSP) + CI check; pure-JVM `Targets` matcher with unit tests.
- Engine is constructible without a device (`Engine::at`), enabling MCP lifecycle tests without hardware.

## Independent review of the changes

A separate reviewer agent audited the diff against the original code. It confirmed:

- the journal and at-most-once semantics are intact
- the download and handshake loops terminate and clean up their forwards
- the Java reformatting changed no behavior

It raised ten issues, all fixed with regression tests:

- stale-after-partial advice that could cause a replay
- errors after a committed action
- wrong-axis scroll fallback
- more than 16 matches reported as a protocol error
- idle-socket `unknown` on the legacy path
- pending-only journal lock-up
- hint-aware `verify` and `wait`
- bracketed labels
- late crop validation
- CLI scale persistence

## Verification

- `cargo xtask verify` passes end to end: version consistency, fmt, clippy `-D warnings`, 71 Rust tests (was 34), npm tests + lint, 14 helper JVM tests (was 9), release APK, docs/source gates, `dist` build.
- End-to-end agent flows (cross-screen chain, missing label, partial failure, passing check, status) run against a scripted fake helper speaking the real wire protocol.
- New tests cover: fake-helper downloads (new and legacy helpers), journal reopen/trim, operation-id scoping, MCP lifecycle without a device, notification/app/label resolution, failure texts, image fitting and coordinate mapping, grammar fuzzing (20k generated commands).
- **Not verified on a real device** (none connected): settle timing, `scroller`, directional scroll, hint resolution, fast handshake. Run `cargo xtask live` and `cargo xtask stress-live` before release.

## Recommended next steps (not done)

1. **Tool names.** `android.read` contains a dot, which the Anthropic and OpenAI function-name regexes disallow; clients must sanitize. Consider adding `android_read` / `android_act` aliases.
2. **JPEG/WebP output.** Palette PNG is ideal for flat UI but a photo-heavy screen at the 350 KB budget is reduced to 16 colors. A lossy encoder would do better for those, at the price of a new dependency, which this project has so far avoided.
3. **On-device screenshot encoding.** PNG at full resolution is the slowest step left (~0.2–0.5 s). Downscaling on-device, or WebP, would cut both capture and transfer time; it needs a receipt field carrying the scale.
4. **`browser.rs` scripts.** The page-observation JavaScript is duplicated between `observe` and `refresh_dom_fingerprint`; unify into one injected script.
5. **Page text paging.** `page text` truncates at about 2.4 KB with no way to continue except `matching`.
6. **Split `Capture.java`** (552 lines) into screen, camera, audio, and projection classes.
7. **Release as 1.1.0.** The helper gained a query (`scroller`) and larger artifact slices. New host + old helper and old host + new helper both work via fallbacks, but the versions should say so.
