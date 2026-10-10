# Agent protocol

MCP advertises two tools. Each takes one string, `command`; the tool descriptions list the common commands.

- `android.read` never changes the device: `status`, `screen`, `screen full`, `screen changes`, `find "TEXT"`, `page`, `page text matching "TEXT"`, `browser tabs`, `notifications`, `location`, `capabilities`.
- `android.act` changes it: `tap "LABEL"`, `type "TEXT" in "FIELD"`, `scroll down`, `open app "NAME"`, `page click "LABEL"`, `wait for text "TEXT" up to 5 seconds`, `verify "LABEL" exists`. Join up to 16 changes with `then`.

Quote labels exactly as `screen` shows them. A duplicated label is listed as `Save (number 2) — button`; send `tap "Save" number 2`. The host owns observation, targets, limits, and the journal. Never construct generations, refs, package names, or JSON plans.

## Results

A successful Android action says what ran, then `Now on screen:` with the settled screen (or the page title and controls after a `page` action). Read again only when you need more detail. Chains may cross screens: a step whose label is not yet visible runs after the earlier steps settle.

| Result | Meaning | Next move |
| --- | --- | --- |
| `Done. …` | Every step ran. | Continue from the screen shown. |
| `No item labeled …` | Not on screen; that step and later ones did not run. | Use a listed label or the attached image. |
| `… matches N items` | Ambiguous at that step; earlier chain steps ran. | Add `number N`. |
| `The screen changed before anything ran` | `stale`. Nothing ran. | Retry the same command. |
| `Check failed` / `Waited …` | A verify or wait was false. | Read, then decide. |
| `Partly done` | Some steps changed the device, then one failed. | Read and reconcile. Never repeat the whole command. |
| `uncertain` | `unknown`: it may have run. | Read before any other change. |
| `missing the Android permission` | Android denied a capability. | Ask the user to grant it on the device. |

## Images and coordinates

When a screen exposes little text, or a label is missing, its image is attached, compressed for you (usually 20–200 KB); the text states its size and scale. `tap point X Y` and `swipe from X1 Y1 to X2 Y2 over MS milliseconds` use coordinates on that image, and the result confirms the device pixel hit. Points outside the image are refused. Any screen change ends the image: capture again before using points, and after rotating. Audio and video are saved to a file path.

## Legacy JSON

The structured JSON forms remain accepted for CLI, JSONL, and MCP callers that do not send `command`. They are deprecated and not advertised. Grammar summary: [installed protocol reference](../../skills/android-use/references/protocol.md).
