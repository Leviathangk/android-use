---
name: android-use
description: Use one enrolled Android device through bounded plain-English read and act commands.
---

# Android Use

Android Use lets an agent read an Android screen or Chrome page, act by visible label, and verify the result.

Decision rule: read only when the current screen or page is unknown; act directly by label when the goal is clear; use `page ...` commands for Chrome content; use a screenshot and coordinates only after a label is missing.

Send these as the `command` string to `android.read` or `android.act` (or run `au read ...` / `au act '...'` in a shell). Quoted values are runtime data for the user's task; there is no default text or target:

```text
status
screen
screen changes
screen matching "TEXT"
find "TEXT"
tap "TARGET"
toggle "TARGET"
type "TEXT" in "FIELD"
scroll down
scroll down in "SCROLL AREA"
open app "DISPLAY NAME"
page open "https://example.invalid"
page text matching "SEARCH TEXT"
page click "TARGET"
page type "TEXT" in "FIELD"
wait for text "EXPECTED TEXT" up to 5 seconds
verify text "EXPECTED TEXT" exists
capture screen
```

Join a short sequence with `then`, outside quotes: `type "TEXT" in "FIELD" then tap "TARGET" then verify text "EXPECTED TEXT" exists`.

Results are short. A successful Android action says what ran and then `Now on screen:` with the settled screen, so a separate read is rarely needed. A duplicated label is listed as `Save (number 2) — button`; send `tap "Save" number 2`. If a label is not on screen, the result lists what is there and attaches the screen image. Nothing runs when the first step's label is missing. When a later step's label is missing, the earlier steps have already run and the result starts with `Partly done`; send only the remaining steps. Use `tap point X Y` on the attached image only when no label works.

`stale` means nothing ran: retry. `partial` means some steps ran: read before another change. `unknown` means it may have run: read and reconcile, never blindly replay. A permission result names the Android approval to ask the user for.

Ask before deletion, purchases, account changes, submissions, notification actions, location, or camera, microphone, and screen recording. Never request shell commands or arbitrary page JavaScript.

Advanced grammar: [protocol](references/protocol.md). Safety: [safety](references/safety.md). Setup: [setup](references/setup.md).
