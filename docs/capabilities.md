# Capabilities

`au capabilities` reports what the connected device and current permission state actually support. Treat that output—not this page—as authoritative for a particular device.

## See the interface

Android Use returns a bounded semantic frontier with visible text, roles, state flags, and interaction references. Ask for more detail only when the frontier is insufficient. Screenshots are private artifacts.

## Act on Android

Commands support tap, long press, text entry, directional scroll (of a labeled area or the screen's main list), system keys, gestures, app launch by display name, bounded waits, and assertions. Each plan names the UI generation it was built from. Stale plans fail before mutation. A successful action returns the settled screen.

## Use Chrome

Chrome control can list and select tabs; observe page titles, text, and interactive elements; navigate; click; focus; type; send keys; scroll; wait; reload; go back or forward; and capture a page screenshot. Chrome must expose its Android debugging socket. Android Use does not expose arbitrary page JavaScript evaluation to agents.

## Inspect supported device state

- Read location when the helper has location permission.
- Read notifications after the user enables Android notification access.
- List notifications by title and text, newest first, and open, dismiss, or invoke the primary action of one by its title (`open notification "TITLE"`).
- Capture rear or front camera images after Android permission.
- Capture bounded mono WAV audio after Android permission.
- Capture a bounded MP4 screen recording after the user grants Android MediaProjection for that process.
- Crop PNG artifacts and compare visual structure through bounded hashes and sampled diffs.

## Not included

Android Use does not include an LLM, cloud device farm, remote broker, root access, silent permission bypass, arbitrary shell tool, or unrestricted JavaScript runtime. App behavior and accessible labels vary by Android version and vendor.
