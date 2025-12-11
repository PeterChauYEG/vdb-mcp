# Vector MCP Server

Semantic code search for Claude Code.

## Quick Start

```bash
./setup /path/to/your/codebase
```

Restart Claude Code and start asking questions about your code.

## Re-indexing

**Automatic**: Install git hooks during setup to auto-reindex on commit/pull/branch switch.

**Manual**: `docker compose --profile index up -d`

## Tools

| Tool | Purpose |
|------|---------|
| `query` | Semantic search with natural language |
| `query_similar` | Find similar code to a reference |
| `trace_path` | Trace execution flows |
| `find_reproduction` | Find bug reproduction steps |
| `map_dependencies` | Map file imports |
| `get_file` | Get complete file content |
| `stats` | Index status |

## Architecture

```
Codebase → Indexer → ChromaDB → MCP Server → Claude Code
```

- Branch-aware: each branch maintains its own index
- Incremental: only re-indexes changed files (SHA-256)
- Respects `.gitignore`

## Requirements

- Docker
- Claude Code
