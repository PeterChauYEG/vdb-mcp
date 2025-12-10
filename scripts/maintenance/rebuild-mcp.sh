#!/bin/bash
# Rebuild the MCP server container after code changes

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

echo "🔨 Rebuilding MCP server container..."
docker build -f Dockerfile.mcp -t vector-mcp-server:latest .

echo ""
echo "✅ MCP server container rebuilt!"
echo ""
echo "⚠️  To use the new version:"
echo "   1. Restart Claude Code (Quit and reopen)"
echo "   2. Or wait ~30 seconds for Claude Code to restart the MCP server automatically"
echo ""
