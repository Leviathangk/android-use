# CLI reference

Run `au setup` once with one authorized, unlocked Android device. Use `au doctor` when anything needs attention. Output is human-readable in a terminal and JSON otherwise; force either with `--human` or `--json`.

| Command | Purpose |
| --- | --- |
| `au setup [APK]` | Enroll the device, install or update the helper, and open the Accessibility screen. |
| `au doctor` | Explain missing device, helper, or permission state, with a resumable `next_step`. |
| `au status` | Quick readiness check. |
| `au read COMMAND` | Same as the `android.read` tool, e.g. `au read screen`, `au read 'find "Wi-Fi"'`. |
| `au act COMMAND` | Same as the `android.act` tool, e.g. `au act 'tap "Wi-Fi"'`. Images are saved and returned as a path. |
| `au serve --mcp` | MCP server over stdio for agent clients. |
| `au serve --jsonl` | JSON Lines over stdio (`{"tool":"android.read","arguments":{"command":"screen"}}`). |
| `au devices` | List connected device endpoints and their states. |
| `au enroll ENDPOINT` | Bind one connected hardware identity when several devices are attached. |
| `au repair [APK]` / `au update [APK]` | Reinstall or update the helper. |
| `au uninstall` | Remove the helper and Android Use's local state. |
| `au version`, `au help` | Version and command list. |

Legacy structured commands remain for compatibility: `au observe [BASE] [--detail]`, `au browser tabs|observe|text`, `au capabilities`, `au location`, `au notifications`, `au visual hash|diff`, `au act JSON` (an argument starting with `{`), and `au artifact ID [START END]`. New integrations should use `read` and `act`.

## Environment

| Variable | Effect |
| --- | --- |
| `AU_ADB` | Absolute path of a trusted `adb` executable. Otherwise `ANDROID_SDK_ROOT`/`ANDROID_HOME` platform-tools, then `PATH`, are searched. |
| `AU_IMAGE_MAX_EDGE` | Upper limit on the longest side of screen images, in pixels (256–4096). Each screen uses the lower of this and its own limit: 1568 dense text, 1280 ordinary UI, 960 sparse. |
| `AU_IMAGE_BUDGET_KB` | Target size of a screen image in KB (32–4096). Default 350; fewer colors, then fewer pixels, are used to meet it. |
| `AU_HOME` | Directory for local state (enrollment, journal, artifacts). Defaults to `%LOCALAPPDATA%\AndroidUse` on Windows and `$XDG_STATE_HOME/android-use` or `~/.local/state/android-use` elsewhere. |

Grammar and results: [agent protocol](agent-protocol.md).
