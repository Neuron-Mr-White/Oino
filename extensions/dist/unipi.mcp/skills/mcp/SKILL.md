---
name: mcp
description: "MCP server management — discover, connect, and use Model Context Protocol tools in Oino"
---

# MCP Tools

MCP (Model Context Protocol) servers provide external tools that extend Oino's capabilities.
Tools from MCP servers are named `{serverName}__{toolName}` — e.g., `github__search_code`.

## Adding Servers

Use `/mcp-add` to add a server: it opens the catalog browser — search the server catalog by
name, description, or category, pick an entry to get a pre-filled config template, or choose
the custom editor to paste a config for an unlisted server. The editor validates the JSON
before saving. The catalog holds 7,800+ servers; a seed catalog (49 servers) is available
offline as fallback.

Project servers are not edited in the overlay — they live in `<project>/.oino/mcp.json`;
edit that file directly.

## Managing Servers

Use `/mcp-settings` for the server registry of configured servers (also reachable from
Settings › Extensions › MCP):

- **Toggle** a server between enabled and disabled
- **Delete** a server from the registry
- **Edit** its config (command, args, env) with validation
- **Scope**: entries are global unless a project defines its own in `<project>/.oino/mcp.json`

Use **"Sync catalog…"** (Settings › Extensions › MCP, or `/mcp-sync`) to refresh the server
catalog from GitHub.

## Quick Status

Use `/mcp-status` for a text summary of all servers and their status.

## Config Hierarchy

MCP config supports **global defaults** with **project overrides**:

```
~/.oino/mcp.json                          ← Core server file (standard MCP format)
~/.oino/state/unipi.mcp/global/mcp.json   ← Servers added from Oino (global)
~/.oino/state/unipi.mcp/global/config.json ← Metadata (enabled/disabled, sync prefs)
{project}/.oino/mcp.json                  ← Project servers (when present, trusted projects)
```

**Merge rules:**
1. Server only in a global file → used normally
2. Server only in the project → used normally
3. Server in both → the project wins entirely
4. A server disabled in the metadata stays disabled

Servers connect over stdio (`command` + `args`) or streamable HTTP (`url`).

### Example server entry

```json
{
  "mcpServers": {
    "github": {
      "command": "docker",
      "args": ["run", "-i", "--rm", "-e", "GITHUB_PERSONAL_ACCESS_TOKEN", "ghcr.io/github/github-mcp-server"],
      "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_xxxx" }
    },
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/home/oi/projects"]
    }
  }
}
```

## Using MCP Tools

Once an MCP server is running, its tools are available as Oino tools.
They're named with the pattern `{serverName}__{toolName}`.

For example, if you add the GitHub MCP server, you'll have tools like:
- `github__search_code`
- `github__create_issue`
- `github__list_pull_requests`

You can call these tools directly in conversations.

## Troubleshooting

### Server won't start
- Check `/mcp-status` for error details
- Verify the command exists: `which npx` or `which docker`
- Check that required env vars are set in the config

### Tools not appearing
- Ensure the server status shows "running"
- Check if the server supports the MCP tools protocol
- Use `/mcp-reload` to reconnect the servers live — no restart needed

### Config issues
- Validate JSON syntax in your config files
- Check file permissions (the `mcp.json` files should be readable)
- Use `/mcp-settings` to view and edit the current configuration

### Catalog sync issues
- Use **"Sync catalog…"** (`/mcp-sync`) to force a refresh
- Check network connectivity to GitHub
- The seed catalog (49 servers) is available offline as fallback
