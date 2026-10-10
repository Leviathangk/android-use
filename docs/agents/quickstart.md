# Agent quickstart

1. Run `au setup` with one unlocked, USB-debugging-authorized device and approve Settings → Accessibility → Android Use. `au doctor` confirms readiness.
2. Register the MCP server: command = absolute path to `au`, arguments = `serve --mcp`. For Claude Code:

   ```bash
   claude mcp add android-use --scope user -- /ABSOLUTE/PATH/TO/au serve --mcp
   ```

   Codex, Cursor, VS Code, and others: [client setup](clients.md).
3. Try it:

```text
android_read: status
android_read: screen
android_act: tap "Settings"
```

Use `page ...` commands for Chrome content. Actions report the resulting screen, so read only when the state is unknown. After `partial` or `unknown`, read before changing anything; never replay an uncertain action. Installation and repair: [install](install.md). Grammar and results: [agent protocol](../reference/agent-protocol.md).
