Use these commands:

```bash
# Claude Code (remote streamable HTTP MCP server)
claude mcp add --transport http train-mcp https://train.floritzmaier.xyz/mcp
```

```bash
# Codex CLI (streamable HTTP MCP server)
codex mcp add train-mcp --url https://train.floritzmaier.xyz/mcp
```

Optional verify commands:

```bash
claude mcp list
codex mcp list
```

Claude Code’s docs show `--transport http` for remote HTTP MCP servers, and Codex CLI’s reference shows `codex mcp add <name> --url <value>` for streamable HTTP URLs. ([Claude][1])

[1]: https://code.claude.com/docs/en/mcp "Connect Claude Code to tools via MCP - Claude Code Docs"
