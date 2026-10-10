# Android Use agent guide

Use the local MCP server `au serve --mcp` ([client setup](docs/agents/clients.md)). It has two tools, each taking one `command` string: `android.read` looks, `android.act` changes the device. Without MCP, run the same commands as `au read screen` or `au act 'tap "LABEL"'`.

Loop: read `screen` (or `page` for Chrome) only when the state is unknown, then act on a label you saw, quoted exactly: `tap "Wi-Fi"`, `type "TEXT" in "FIELD"`, `scroll down`, `open app "NAME"`. Every successful Android action reports `Now on screen:`, so you rarely need another read. Join a few steps with `then`. Use `number N` for a duplicated label.

Use the attached screen image and `tap point X Y` only after a label is missing. After `stale`, retry; after `partial` or `unknown`, read and reconcile before any other change. Ask before destructive, account, purchase, submission, notification, location, or camera, microphone, or recording actions.

Details: [agent protocol](docs/reference/agent-protocol.md), [setup](docs/agents/quickstart.md), [security](SECURITY.md).
