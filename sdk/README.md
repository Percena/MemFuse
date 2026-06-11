# @percena/memfuse

SDK for MemFuse — the persistent memory hub for AI coding agents.

## Installation

```bash
npm install @percena/memfuse
```

## Prerequisites

MemFuse Server must be running. Download from [GitHub Releases](https://github.com/Percena/MemFuse/releases) or build from source:

```bash
cargo build --release -p mfs-server && ./target/release/mfs-server
```

## What's Included

| Component | Entry Point | Description |
|-----------|-------------|-------------|
| **CLI** | `npx --package=@percena/memfuse memfuse` | 110 commands covering all API operations |
| **MCP Server** | `npx --package=@percena/memfuse memfuse-mcp` | 43 Agent-facing tools via MCP protocol |
| **Setup Tool** | `npx --package=@percena/memfuse memfuse-setup` | Platform installer for Claude Code / Codex |
| **Lifecycle Hooks** | `@percena/memfuse/hooks` | 8 hooks for Claude Code, 3 for Codex |
| **HTTP Client** | `@percena/memfuse/client` | Type-safe client for MemFuse Server API |

## Quick Start

### 1. Start the Server

```bash
./run-server.sh
# Verify: curl http://127.0.0.1:18720/health
```

### 2. Set Up for Your Agent Platform

```bash
# Claude Code
npx --package=@percena/memfuse memfuse-setup install --platform=claude-code --server-url=http://127.0.0.1:18720

# Codex
npx --package=@percena/memfuse memfuse-setup install --platform=codex --server-url=http://127.0.0.1:18720
```

### 3. Use the CLI

```bash
# Search memories
npx --package=@percena/memfuse memfuse search --query "auth decisions" --strategy diverse

# List facts
npx --package=@percena/memfuse memfuse list-facts

# Store an observation
npx --package=@percena/memfuse memfuse store-observation --tool-name "discovery" --content "Found rate limiter config"

# Check health
npx --package=@percena/memfuse memfuse health
```

Or install globally:

```bash
npm install -g @percena/memfuse
memfuse search --query "auth decisions"
```

### 4. Use as MCP Server

Add to your agent's MCP configuration:

```json
{
  "mcpServers": {
    "memfuse": {
      "command": "npx",
      "args": ["--yes", "--package=@percena/memfuse", "memfuse-mcp"],
      "env": {
        "MEMFUSE_SERVER_URL": "http://localhost:18720",
        "MEMFUSE_USER_ID": "your-user-id"
      }
    }
  }
}
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MEMFUSE_SERVER_URL` | `http://127.0.0.1:18720` | MemFuse Server URL (canonical default everywhere) |
| `MEMFUSE_USER_ID` | `default` | User identifier |

## Links

- [MemFuse Server](https://github.com/Percena/MemFuse) — The Rust server
- [Architecture](https://github.com/Percena/MemFuse/blob/main/docs/architecture.md) — System design and crate structure
- [User Guide](https://github.com/Percena/MemFuse/blob/main/docs/user-guide.md) — Detailed usage guide

## License

MIT
