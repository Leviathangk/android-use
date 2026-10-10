# Android Use: final master plan

This plan merges three sources:

- the earlier planning package (`android-use-full-plan.md`: master roadmap, emulator plan, and redesign proposal), written against `main` at `5509535`;
- the work done in this session on branch `improve/agent-native-reliability`, which starts from the same `5509535` and is not committed yet;
- two multi-agent code reviews: 50 confirmed findings, then a regression pass that is still running.

When this file and the older package disagree, **this file wins**. Its line references are to the working tree, not to `5509535`.

---

## 0. Status snapshot (2026-10-09)

| | |
| --- | --- |
| Baseline | `main` @ `5509535` (1.0.0) |
| Working branch | `improve/agent-native-reliability`, about 50 files modified, **nothing committed yet** |
| Rust tests | 92 pass (unit and integration) |
| JVM tests | 15 pass |
| Lint and format | clippy clean; `google-java-format --aosp` clean; docs gate passes |
| Failing gate | **Size only.** Rust production is 6,816 lines against a limit of 6,700 |
| Open process items | The regression review workflow is still running. The scratch folder is deleted when the session ends, so the work must be committed or moved |
| Toolchain installed | Homebrew OpenJDK 21; Gradle 9.1.0 (verified with a checksum); Android SDK (license accepted); google-java-format 1.37.0 |

Current budgets compared with the old plan:

| Budget | Old (at `5509535`) | Now | Limit now |
| --- | --- | --- | --- |
| Rust production | 5,620 / 6,500 | **6,816** | 6,700 (needs a decision; see §4) |
| Java | 1,011 / 2,200 | ~3,016 (formatted with `--aosp`, so a bigger line count is expected) | 3,200 |
| Authored code overall | — | under budget | 13,000 |
| Largest Rust file | `api.rs` at 900 / 900 | `browser.rs` at 799 | 900 per file |

The size gate now counts `#[cfg(test)]` code as tests (`rust_split` in `tools/src/main.rs`), so moving tests out of production files frees budget.

---

## 1. Plan item status

Status values: **Done** (implemented and covered by a test), **Partly** (in progress), **Open** (not started), **Changed** (the plan was revised by what we built).

### 1.1 Correctness defects (Track A / redesign §2.2)

| # | Item (old ref) | Status | What exists now |
| --- | --- | --- | --- |
| A1 | Synthetic `unknown` receipt was missing `g`/`m` (§2.2-1) | **Done** | Unknown outcomes now reach the agent with read-and-reconcile guidance. Dispatch errors are journaled as `Done unknown`. A lost Chrome reply also reports unknown, and New/Close report unknown when they cannot confirm |
| A2 | Chains kept running after stale, partial or unknown (§2.2-2) | **Done** | `agent.rs` stops at the first plan that does not finish. `Partly done …` lists only the remaining steps |
| A3 | `isError:false` was hard-coded (§2.2-3) | **Done** | `isError` follows `response.failed`; a partial result sets `failed` (`adapter.rs:124`) |
| A4 | Silent substring targeting (§2.2-4) | **Partly** | `resolve.rs` is now the single rule: an exact match (ignoring case and whitespace) beats a substring; two or more matches is ambiguous; `number N` picks one. **Open:** a single substring match still acts. Decide whether to (a) act and say "tapped *Save draft* (closest to Save)" or (b) return `not_found` with suggestions. Recommendation: (a) for one match with high overlap, otherwise (b) |
| A5 | Only the active window is read (§2.2-5) | **Open** | `Ui.java:335` and `Vm.java:441` still call `getRootInActiveWindow`. See P4 |
| A6 | No check that an action had an effect, and no settle wait (§2.2-6) | **Partly** | The host settles: it polls the UI generation until quiet, within a limit, then returns `Now on screen:`. **Open:** gestures still use a null callback; the device-side settle and diff are deferred to P2 |
| A7 | Silent truncation (§2.2-7) | **Partly** | The omitted-count lines exist (`format.rs:132-135, 245`). **Open:** make sure the count survives byte truncation by placing it before the body; add device-side `truncated` flags |
| A8 | Microphone capability always reported true (§2.2-8) | **Open** | `Capture.java:85` still reports `AUDIO_SERVICE != null`. Fix: check `PackageManager.FEATURE_MICROPHONE` |
| A9 | Every JSON-RPC error returned `-32602`, and `ping` failed (§2.2-9) | **Done** | `ping` answers `{}`; unknown methods return `-32601`. Both are tested |
| A10 | Fuzzing never reached the parsers (§2.2-10) | **Partly** | The grammar tests moved to `computer/tests/command_grammar.rs` and `protocol_fuzz.rs` was updated. **Open:** fuzz `parse_act_command` itself with structured random commands |
| A11 | Docs and parser grammar disagreed (§2.2-11) | **Partly** | The protocol, CLI, SKILL and architecture docs were rewritten against the parser. **Open bug:** the docs say links are HTTP(S) only, but `api.rs:418` accepts `geo:` and `google.navigation:`. Either change the docs or drop the schemes. Recommendation: keep the schemes and fix the docs |
| A12 | Operation ids repeated across MCP sessions (J5, D9) | **Done** | Operation ids are salted per process (`engine.rs:35-37, 334-346`), with a test |
| A13 | No lock across processes on the journal (A.3-3) | **Done** | `File::lock`, a reload on every access, repair of a torn tail, single-write append with rollback, and pending records are never evicted |
| A14 | Pre-send `Pending` records were never evicted (§2.2 unverified) | **Done (Changed)** | Pending records now **fence**: a new action returns `unknown` until a read reconciles them |
| A15 | Wrong device chosen when two transports share a serial (J4, D7) | **Open (critical)** | `device.rs:133-153` still returns the **first** match. This is the highest-priority remaining fix |
| A16 | No `sys.boot_completed` wait (J7) | **Open** | Nothing in `computer/src` checks it |
| A17 | Foreground-service types on API 34+ (J3) | **Partly** | `projectionStarted()` now passes `MEDIA_PROJECTION` (plus `SPECIAL_USE` on 34+). **Open:** `onCreate` calls `startForeground(31, note)` with no type (`BridgeService.java:28`), and camera and microphone types are missing. Must be checked on an API 34+ device or emulator |
| A18 | State files had no 0700/0600 modes (C.4) | **Open** | No `set_permissions` anywhere |
| A19 | Browser label fallback could expose typed values (§2.2 unverified) | **Open (verify)** | Check `browser.rs` label fallback |
| A20 | Idle helper socket times out after 35 s, so the next act is `unknown` | **Partly** | Reconnect on transport errors plus the fast handshake reduce this. **Open:** a keepalive, or reconnecting before send, so an idle timeout never produces `unknown` |

### 1.2 Fixes found by this session's review (not in the old plan)

All of these are **Done** and covered by tests:

- **Helper commit-then-throw** returns `R(3, …)` as `partial`, not a failure.
- **Nearest-control fallback removed.** `spatialOwner` only accepts a control that contains the point.
- **Horizontal scroll** never falls back to forward or backward scrolling.
- **`Pred` ref predicates** store the scene generation and fail closed after a change.
- **Label normalization** is identical on host and helper (`Targets.java` + `TargetsTest`): Unicode spaces and trailing ellipses.
- **Notifications** use one label (the title, or the text when untitled) for both listing and targeting.
- **Tabs:** only `page` targets are listed.
- **Owner ordering:** owners resolve top to bottom, then left to right, matching how the screen is numbered.
- **Images:**
  - a stale image is dropped;
  - `ImageSpace` is cleared after any change;
  - `map_point` refuses when no image is current;
  - only one media capture is allowed per command;
  - photo crops give a clear JPEG error.
- **Classifier** compares vertical neighbours, so tile grids cannot slip through the sampling stride.

### 1.3 Speed and agent-nativeness (old §2.1 / §2.3)

| Item | Status | Notes |
| --- | --- | --- |
| Artifact chunks of 2,800 bytes (~375 round trips per MiB) | **Done (Changed)** | 256 KiB slices (`Capture.MAX_CHUNK`), with a 2,800-byte fallback for old helpers |
| Lossless PNG q100 screenshots | **Done (Changed)** | `compress.rs` classifies content, resizes with area averaging, then quantizes to an indexed PNG within a 350 KB byte budget. Typical cost is 25–35 ms. `ImageSpace` maps image coordinates back to device pixels. **This replaces the redesign's JPEG/"delete visual.rs" idea**: `png` stays a dependency, and `visual::decode` is shared |
| A read was needed after every act | **Done** | Act results include the settled `Now on screen:` (or `Now:` for pages) |
| Helper connection and service start | **Done** | Lazy, persistent link; one reconnect; a running helper is reused; fast handshake |
| Repeated app-list queries | **Done** | The helper caches the app list for 30 s |
| `au read` / `au act` CLI | **Done** | The same grammar as MCP; JSON when stdout is not a terminal |
| Client setup docs | **Done** | `docs/agents/clients.md` (Claude Code, Codex, Cursor, Claude Desktop, VS Code) |
| `structuredContent` for agent results | **Partly** | Used for the legacy JSON path (`adapter.rs:86`). **Open:** emit status, resolved target and generation for `android.read` and `android.act` too (P2) |
| Element handles, `scroll until`, multi-window | **Open** | P4 |
| One device round trip per intent, with diff | **Open** | P2 |
| adb smart-socket, daemon, HMAC | **Open** | P3 |

### 1.4 Simplification (Track S)

| Item | Status |
| --- | --- |
| Split `api.rs` (it was at the 900-line cap) | **Done.** Legacy parsing moved to `legacy.rs`; agent tests moved to `agent_tests.rs` |
| Split the old `engine.rs` | **Done.** `engine.rs` (core), `agent.rs` (agent layer), `format.rs`, `resolve.rs`, `journal.rs`, `compress.rs` |
| Delete legacy JSON | **Open; blocked by Q2.** `tools/src/main.rs:223-322` still sends `{"op":…}` requests |
| Delete `visual.rs` and the `png`/`sha1` dependencies | **Dropped.** Compression needs `png`. Re-evaluate `sha1` only |
| `spec.rs`, generating the docs and tool description | **Open** |
| Remove dead code (`visual::Op`, `api::Req`, `SDK_INT<21` guards) | **Open (verify each)** |
| Oxlint plugin, and its missing notice in THIRD_PARTY_NOTICES | **Open** |

### 1.5 Maintenance (Track M)

| Item | Status |
| --- | --- |
| CI job that checks helper formatting | **Done** (`helper-format` in `ci.yml`) |
| Close stale Dependabot PRs #10–#18 (paths deleted in `a03e1b5`) | **Open.** This is outward-facing, so it needs the owner's approval |
| Review the live Dependabot PRs #20–#37 and #43–#44 | **Open** |
| CHANGELOG `Unreleased` | **Done.** Update it again when the remaining phases land |

---

## 2. Decisions

The old decision register (D1–D15, R1–R5, U1–U6) still stands, with these changes:

| # | Decision | Status now |
| --- | --- | --- |
| D8 | OS file lock per target | **Partly realized.** The journal already uses `File::lock`. A target lock should use `try_lock` so a second `serve` fails fast with "in use" |
| D9 | Session nonce in operation ids | **Done.** The target identity is still **not** included in the plan digest; that comes with E2 |
| D16 *(new)* | Screenshots: indexed PNG plus `ImageSpace`; JPEG only for camera photos | **Implemented** |
| D17 *(new)* | Pending journal records fence new actions until a read | **Implemented.** This replaces "never auto-replay" with "refuse until reconciled" |
| D18 *(new)* | Unit tests live outside production files so the size gate counts them as tests | **Implemented** (`rust_split`) |
| Owner-1 | Fail-closed resolution and the nonce come before feature work | The nonce is **done**; fail-closed resolution is **still needed** (A15) |
| Owner-2 | When to remove legacy JSON, or raise the Rust budget | **More urgent now**: the branch is 116 lines over. See §4 |
| Owner-3 | Remote broker (E7) | Unchanged; needs approval |
| Owner-4 | Keep `applicationId` `dev.codex.aubridge` | Unchanged; recommended |
| Owner-5 | Accessibility provisioning only on emulators | Unchanged; needs approval |

---

## 3. Final plan of edits, in order

Each step can ship on its own. File references are to the working tree.

### Step 0: Land what exists (this branch)

1. Wait for the regression review workflow and fix anything it confirms.
2. **Budget:** pick one of the options in §4. Recommendation: for this PR, raise the Rust production limit from 6,700 to 6,900 in `tools/src/main.rs:564`, with a CHANGELOG note. That pays for the journal, compression and correctness work. Also commit to the S1 deletions so the limit can come back down.
3. Run the full gate: `cargo xtask verify`, Gradle unit tests, and `gjf --aosp --dry-run`.
4. Commit in logical slices:
   1. journal and engine split;
   2. agent layer and resolution;
   3. helper fixes;
   4. compression;
   5. docs and CI.
5. Open a PR from `improve/agent-native-reliability`.
6. Copy the repo out of the scratch folder.

### Step 1: Remaining correctness (Track A remainder), ~1 PR

| Edit | File | Accept |
| --- | --- | --- |
| **A15 fail-closed resolution.** Try the stored endpoint first; otherwise collect *all* transports whose serial matches. One match binds; more than one returns `Code::Ambiguous` with an `au enroll ENDPOINT` hint | `device.rs:133-153` | Unit test with a fake device list that includes duplicates |
| **A16 boot wait.** Add `wait_boot_completed(serial, timeout)` (poll `getprop sys.boot_completed` at 1 Hz) and call it from `setup` and `doctor` | `device.rs`, `install.rs` | Fake adb: boot completes after N polls |
| **A17 foreground-service types.** `onCreate`: `startForeground(31, note, FOREGROUND_SERVICE_TYPE_SPECIAL_USE)` on API 34+, plus the `<property>` declaration for special use in the manifest; acquire the projection only after the foreground service has the media-projection type | `BridgeService.java:28`, `Capture.java`, `AndroidManifest.xml` | Screen recording works on an API 34+ emulator |
| **A8 microphone capability** via `hasSystemFeature(FEATURE_MICROPHONE)` | `Capture.java:85` | JVM test |
| **A11 link schemes.** Change the docs to "HTTP(S), `geo:`, `google.navigation:`", or drop the two schemes | `api.rs:418`, `docs/reference/agent-protocol.md`, `docs/concepts/architecture.md`, SECURITY.md | Docs gate plus a grammar test |
| **A4 single substring match.** Name the resolved label in the receipt, or return `not_found` with suggestions | `resolve.rs`, `format.rs` | Test: `tap "Save"` with only "Save draft" present |
| **A7 truncation.** Put the omitted count first; device-side `truncated` flag | `format.rs`, `Ui.java` | Test with an oversized screen |
| **A18 permissions.** Directories 0700, files 0600 on Unix | `device.rs` (`Paths`, `atomic`), `artifact.rs`, `journal.rs` | Unix unit test |
| **A19** Review and test the browser label fallback | `browser.rs` | Test: a typed password never appears |
| **A20** Reconnect before send after idle (track the last-used time), so idle never turns into `unknown` | `engine.rs`, `bridge.rs` | Test: a 40 s idle followed by an act returns `done` |
| **Remaining low-severity items** | `engine.rs`, `adapter.rs`, `device.rs`, `format.rs` | One test each. The items:<br>• screenshot fallback label<br>• the connect deadline is enforced<br>• no adb forward leaks on parse failure<br>• end-of-list scroll message<br>• stray `null` when `serve` exits<br>• render escaping<br>• numbering for duplicate group rows |

### Step 2: Shared test infrastructure (Q1, Q2)

1. **Q1: fake adb plus a fake helper.**
   - Add `computer/tests/support/fake_adb.rs`: a scripted binary that `AU_ADB` points at. It covers:
     - duplicate serials;
     - port moves;
     - boot not yet complete;
     - `id -u` = 0;
     - a forward failure.
   - Add a fake helper that speaks the frame protocol, covering:
     - the handshake;
     - reconnect;
     - a sequence mismatch;
     - receipt mapping;
     - commit-then-throw.
2. **Q2: port the harnesses.** Rewrite `xtask live`, `stress-live` and `benchmark-live` (`tools/src/main.rs:187-466`) to send `{"command":"…"}` through `android.read` and `android.act`. Point them at `device/example` (`aubench`) and a local static page instead of the internet.
3. **A10:** fuzz `command::parse_*` with random structured commands (quotes, `then`, numbers, Unicode).

### Step 3: Simplify and delete (S), after Q2

1. Delete the legacy JSON plan protocol: `legacy.rs`, its CLI forms (`au observe`, `au browser …`, `au act {…}`, `au artifact`), and the shim if it is unused. That is about −650 lines, which brings Rust production back well under 6,500.
2. Remove the dead code listed in §1.4, checking each item before deleting it.
3. **`spec.rs`:** one table of verbs, arguments and bounds. It generates:
   - the MCP tool description;
   - `skills/android-use/references/protocol.md`;
   - the grammar test corpus.

   `xtask docs` fails when the generated output differs.
4. Remove the oxlint plugin, or cut it to two rules; add the THIRD_PARTY_NOTICES entry.
5. Lower the Rust budget back down, to 6,500 or less.

### Step 4: Targets and identity (E2)

1. Add `target.rs` with:
   - the target record (schema 1);
   - name validation `[a-z0-9-]{1,32}`;
   - `targets/<name>/` directories;
   - a `try_lock` per target;
   - 0700/0600 modes.
2. Add a global `--target NAME` / `AU_TARGET` (`main.rs`). With neither, behavior is byte-identical to today; check with golden CLI output.
3. `Engine::open(target)`. The digest becomes `sha256(target identity ‖ plan)`. A mid-session reset re-resolves only the same target.
4. `install::*` become target-scoped; `uninstall` gets a scope.
5. Tests: `computer/tests/target_identity.rs`, using Q1.

### Step 5: Emulator (E1, E3–E7)

The old plan's Parts 2 and C–K stand as written. These items change:

- **E1 (can start now):**
  - Build the `xtask emulator-e2e` skeleton (J1).
  - Answer U1–U6 (J2).
  - The SDK is now installed on this Mac, but an emulator package and a system image are still needed: `sdkmanager "emulator" "system-images;android-36;google_apis;arm64-v8a"`.
  - Validating the A17 foreground-service fix moves into E1.
- **E3 budget:**
  - `emulator.rs` is about +420 lines, plus about +420 elsewhere.
  - It can land only after Step 3's deletions, or with an explicit raise to about 7,400.
  - `api.rs` is no longer at its cap, but new code still goes in `emulator.rs`.
- **E4:** `--provision` with the C.8 guards; it depends on the owner's approval (Owner-5).
- **E5–E7:** unchanged.

### Step 6: Performance and protocol (P2–P4)

- **P2:**
  - Resolve, act, settle (with the gesture completion callback) and diff on the device, in one round trip.
  - Emit `structuredContent` (status, resolved target, generation, `changed`) for `android.read` and `android.act`.
  - The host-side `settle()` in `agent.rs:250` then becomes a fallback for old helpers.
- **P3:**
  - an adb smart-socket client (TCP 5037);
  - blob frames, which replace chunked artifact fetches entirely;
  - daemon reuse;
  - HMAC authentication with a secret provisioned at setup;
  - CDP over the same stream.

  Starts after E2.
- **P4:**
  - `getWindows()` with dominance rules, which fixes A5;
  - a narrower event mask instead of `typeAllMask`;
  - `flagIncludeNotImportantViews` only for `screen full`;
  - stable keys and `eN` handles;
  - `scroll until "X"`.

  Also: set-of-marks overlays drawn on the compressed `ImageSpace` image, so visual fallback produces `tap eN` rather than raw points.

---

## 4. Budget decision (needed for Step 0)

| Option | Effect |
| --- | --- |
| **A (recommended).** Raise Rust production to 6,900 now. Step 3 then deletes about 650 lines and the limit drops to 6,500 | Ships today's work as is. The raise is temporary and recorded |
| B. Cut about 120 lines now: move more `#[cfg(test)]` blocks out, and drop low-use legacy CLI forms early | No policy change, but it partly removes legacy code before Q2 |
| C. Raise straight to 7,400 (the old plan's §5d) | Covers emulator work too. Not recommended until Step 3 has been tried |

---

## 5. Priority order

1. **Step 0:** land the branch (budget A, PR).
2. **Step 1:** A15 fail-closed resolution first. It is the only remaining critical defect. Then A16, A17 and A11.
3. **E1 prototype and Q1**, in parallel.
4. **Q2, then S.** This removes legacy code and frees budget.
5. **E2, then E3 and E4.**
6. **P2:** the biggest remaining speed win for agents.
7. **E5, P3, P4.**
8. **E6.**
9. **E7**, only with approval.

## 6. Release gates

Unchanged from the old plan's §K, plus:

- **Compression:** p95 under 50 ms on a 1080×2400 capture, and the output stays within the byte budget. Add this to `benchmark-live` once Q2 lands.
- **Journal crash injection:** kill −9 between pending and done, then confirm the next act returns `unknown` until a read, and that nothing is replayed.
