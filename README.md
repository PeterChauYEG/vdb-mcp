# Vector MCP Server

**Semantic code search for Claude Code.** Ask questions about your codebase in natural language.

## Quick Start

```bash
./setup /path/to/your/codebase

# Setup waits for indexing to complete automatically
# Then restart Claude Code and start asking questions!
```

The setup script handles:
- ✅ Configuration (.env file)
- ✅ Starting indexing (ChromaDB + indexer)
- ✅ Claude Code integration (.mcp.json)
- ✅ Optional git hooks for auto-reindexing

## Usage

Ask Claude questions about your code.

The MCP returns **complete code** - Claude doesn't need to read individual files.

## Re-indexing

### Automatic (Recommended)
During setup, you can optionally install git hooks that automatically re-index after:
- `git commit` (via post-commit hook)
- `git pull` (via post-merge hook)

The hooks are local-only (not checked into git) and can be removed anytime by deleting them from `.git/hooks/`.

### Manual
```bash
docker compose up -d
```

### Branch-Based Indexing

**The index is branch-aware** - each git branch maintains its own index at the latest commit:

- **Switching branches**: Queries automatically return results for your current branch only
- **Content hashing**: Files are re-indexed only when content actually changes (SHA-256)
- **One commit per branch**: Old commits are automatically cleaned up when you update a branch
- **Bounded storage**: Typically ~5 branches × 10k chunks = 50k total documents

**How it works**:
1. Each code chunk stores `git_branch`, `git_commit`, and `content_hash` in metadata
2. When you commit or pull, only files with changed content are re-indexed
3. Old commits for the same branch are automatically deleted
4. All queries filter by your current branch - you only see code from your active branch

**Example workflow**:
```bash
# On develop branch - index is current
git checkout feature/new-auth
docker compose up -d  # Indexes feature branch

# Ask Claude: "where is authentication?"
# Returns results from feature/new-auth only

git checkout develop
# Claude now queries develop branch index
```

## Features

- **Branch-aware indexing**: Each git branch maintains separate index at latest commit
- **Content hashing**: SHA-256 hashing detects actual file changes (not just commits)
- **Automatic branch filtering**: Queries only return code from your current branch
- **Smart chunking**: 2000-char chunks with overlap for complete context
- **Documentation**: Indexes `.md`, `.mdx`, `.txt`, `.rst`, `.adoc`
- **Portable**: Each developer configures their own local path
- **Incremental**: Only re-indexes changed files based on content hash
- **Bounded storage**: Old commits automatically cleaned up (~50k chunks total)
- **7 Powerful MCP tools**: Simplified, Claude-powered semantic code understanding

## Tools Available

| Tool | Use Case |
|------|----------|
| `query` | 🔍 Semantic search - Find ANY code using natural language |
| `query_similar` | 🔗 Find similar code to a reference file |
| `trace_path` | 🛤️ Trace execution flows (how to navigate to X screen?) |
| `find_reproduction` | 🐛 Find steps to reproduce bugs/features |
| `map_dependencies` | 📊 Map file dependencies and imports |
| `get_file` | 📄 Get complete file with git hash validation |
| `stats` | 📊 Check index status and freshness |

**Why only 7 tools?** The old 11 tools had massive redundancy - most just tweaked the query string before calling the same search. Now `query` handles everything, and Claude crafts the right queries. This is simpler, more powerful, and leverages Claude's intelligence.

## Architecture

```
Your Codebase → Indexer (Python) → ChromaDB → MCP Server (Node.js) → Claude Code
                    ↓
            Embeddings stored locally
            Respects .gitignore (no hardcoded patterns)
            Tracks git branch + commit
            Content-based change detection
```

**Branch-Based Storage Model**:
- Each branch stores only its **latest commit**
- Content hashing (SHA-256) detects actual changes
- Queries automatically filtered by current branch
- Old commits deleted on branch update
- Storage: ~5 branches × 10k chunks = 50k total

**Team Setup**: Each developer runs `./setup` with their own local path. The `.env` file (gitignored) stores per-developer config including current branch. Everything else is shared via git.

## Scripts & Maintenance

### Setup Scripts
- `./setup <path>` - One-command setup (does everything)

### Maintenance Scripts
- `./scripts/maintenance/rebuild-mcp.sh` - Rebuild MCP server container
- `./scripts/maintenance/clear-and-reindex.sh` - Clear index and start fresh
- `./scripts/maintenance/reset.sh` - Complete reset (containers, data, config)

### Core Files
- `mcp-server.sh` - MCP server entrypoint (called by Claude Code)
- `scripts/index_codebase.py` - Indexer implementation
- `compose.yaml` - Container orchestration

## Requirements

- Docker
- Node.js 20+ (for MCP server)
- Python 3.11+ (for indexer)
- Claude Code

## FAQ

### How does branch switching work?

When you switch branches in your codebase:
1. The git hooks (if installed) trigger re-indexing via `docker compose up -d`
2. The indexer detects the new branch and checks if it's already indexed
3. If not indexed, it processes all files for that branch
4. If already indexed at the current commit, it skips indexing
5. The MCP server automatically filters all queries by the current branch

### What happens to old commits?

When you commit or pull on a branch, the indexer:
1. Detects the new commit hash
2. Deletes all chunks from the previous commit on that branch
3. Indexes only files with changed content (via SHA-256 hash comparison)
4. Stores the new chunks with the latest commit hash

This keeps storage bounded - each branch only stores its latest state.

### Can I have multiple branches indexed?

Yes! The system supports multiple branches simultaneously. Each branch maintains its own index. When you query, you only see results from your current branch. Typical setup: ~5 branches × 10k chunks = 50k total documents.

### What if I don't use git hooks?

You can manually trigger re-indexing with `docker compose up -d` after switching branches or making changes. The indexer is smart enough to only process changed files.

### How does the indexer know what to ignore?

The indexer **respects your .gitignore file** - no hardcoded patterns. It will skip any files or directories listed in `.gitignore`. The only hardcoded ignores are:
- `.git/` directory (never index git internals)
- `.DS_Store` files (macOS metadata)

This means the indexer is universal - it works with any codebase without customization.

## Known Issues

< 0.1% of files may re-index on every run (harmless ChromaDB quirk with floating-point metadata). 99.9%+ of files correctly skip when unchanged.

docker compose build indexer --no-cache
rm -rf ./data/chroma
./setup ~/src/laboratory-one/ro/bot
