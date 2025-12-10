#!/bin/bash
# Wrapper script to run MCP server via Docker
# This allows Claude Code to start the server on-demand

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Load configuration from .env if it exists
if [ -f ".env" ]; then
    source ".env"
fi

# Ensure the MCP server image is built
if ! docker image inspect localhost/vector-mcp-server:latest >/dev/null 2>&1; then
    echo "Building MCP server image..." >&2
    docker build -f Dockerfile.mcp -t vector-mcp-server:latest . >&2
fi

# Run the MCP server container with stdio passthrough
docker run --rm -i \
  --network host \
  -e CHROMA_HOST=localhost \
  -e CHROMA_PORT=8000 \
  -e COLLECTION_NAME="${COLLECTION_NAME:-codebase}" \
  -e CODEBASE_PATH="${CODEBASE_PATH}" \
  -e GIT_BRANCH="${GIT_BRANCH}" \
  localhost/vector-mcp-server:latest
