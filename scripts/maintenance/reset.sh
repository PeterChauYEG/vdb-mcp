#!/bin/bash
# Complete reset - removes all containers, images, data, and configuration

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

echo "🧹 Complete Vector MCP Reset"
echo ""
echo "This will remove:"
echo "  • All containers (chromadb, indexer)"
echo "  • Container images"
echo "  • All indexed data (./data/chroma/)"
echo "  • Configuration (.env)"
echo ""
read -p "Continue? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Cancelled."
    exit 0
fi

echo ""
echo "🛑 Stopping and removing containers..."
docker compose down -v 2>/dev/null || true

echo "🗑️  Removing container images..."
docker rmi vector-mcp-server:latest 2>/dev/null || true
docker rmi -f $(docker images -q vector-mcp-indexer) 2>/dev/null || true

echo "💾 Removing indexed data..."
rm -rf data/chroma/*

echo "📝 Removing configuration..."
rm -f .env

echo ""
echo "✅ Reset complete!"
echo ""
echo "To start fresh, run:"
echo "  ./setup /path/to/codebase"
echo "  docker compose up -d"
echo "  ./scripts/setup/configure-claude.sh"
echo ""
