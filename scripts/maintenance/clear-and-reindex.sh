#!/bin/bash
# Clear the index and start fresh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$PROJECT_ROOT"

echo "🗑️  Stopping and removing indexer container..."
docker stop vector-mcp-indexer 2>/dev/null || true
docker rm vector-mcp-indexer 2>/dev/null || true

echo "🗑️  Deleting ChromaDB data..."
rm -rf ./data/chroma/*

echo "✅ Index cleared!"
echo ""
echo "To re-index, run:"
echo "  docker compose --profile index run --rm indexer"
