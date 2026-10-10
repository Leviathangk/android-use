# Connect an agent client

Android Use is a local stdio MCP server. Every client needs the same two facts:

- **command:** the absolute path to `au` (`au.exe` on Windows), kept beside `aubridge.apk`
- **arguments:** `serve --mcp`

The server starts even when no device is connected; tool results then explain what to fix. Run `au setup` once before first use (see the [human quickstart](../getting-started.md)).

Replace `/ABSOLUTE/PATH/TO/au` below with your path. On Windows use `C:\\path\\to\\au.exe` inside JSON strings.

## Claude Code

```bash
claude mcp add android-use --scope user -- /ABSOLUTE/PATH/TO/au serve --mcp
```

Or commit a project-scoped `.mcp.json`:

```json
{ "mcpServers": { "android-use": { "command": "/ABSOLUTE/PATH/TO/au", "args": ["serve", "--mcp"] } } }
```

## Codex

Add to `~/.codex/config.toml`:

```toml
[mcp_servers.android-use]
command = "/ABSOLUTE/PATH/TO/au"
args = ["serve", "--mcp"]
```

## Cursor and Claude Desktop

Cursor reads `~/.cursor/mcp.json` (or `.cursor/mcp.json` in a project); Claude Desktop reads `claude_desktop_config.json`. Both use:

```json
{ "mcpServers": { "android-use": { "command": "/ABSOLUTE/PATH/TO/au", "args": ["serve", "--mcp"] } } }
```

## VS Code

Add to `.vscode/mcp.json`:

```json
{ "servers": { "android-use": { "type": "stdio", "command": "/ABSOLUTE/PATH/TO/au", "args": ["serve", "--mcp"] } } }
```

## Any other MCP client

Use a stdio transport with the command and arguments above. The server implements MCP revisions 2025-06-18, 2025-03-26, and 2024-11-05, answers `ping`, and sends operating `instructions` during `initialize`. It exposes two tools:

| Tool | Use |
| --- | --- |
| `android.read` | Look without changing anything: `screen`, `page`, `notifications`, … |
| `android.act` | Change the device: `tap "Wi-Fi"`, `type "hello" in "Message"`, `page open "https://…"`, … |

Each takes one string argument, `command`. Some clients show the names with dots replaced (for example `android_read`); they are the same tools. The tool descriptions list the common commands, so an agent can work from them alone; the [agent protocol](../reference/agent-protocol.md) explains results in more depth.

## Agents without MCP

Any agent that can run shell commands can use the same grammar through the CLI. Output is JSON when stdout is not a terminal:

```bash
au read screen
au act 'tap "Wi-Fi"'
au act 'type "hello" in "Message" then tap "Send"'
```

Images an agent asks for (for example `au act 'capture screen'`) are saved under Android Use's private artifact folder and returned as a file path.

## Check the connection

Ask the agent to run `android.read` with `status`, then `screen`. A healthy setup answers `Ready. Android Use is connected…` followed by the visible controls. If not, `au doctor` on the computer explains the next step.
