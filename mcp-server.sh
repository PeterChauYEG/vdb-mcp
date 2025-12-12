#!/bin/bash
# Wrapper script to run MCP server via Docker
# This allows Claude Code to start the server on-demand

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Parse command line arguments
COLLECTION_NAME=""
CODEBASE_PATH=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --collection)
            COLLECTION_NAME="$2"
            shift 2
            ;;
        --codebase)
            CODEBASE_PATH="$2"
            shift 2
            ;;
        *)
            shift
            ;;
    esac
done

# Fallback to .env for backwards compatibility
if [ -z "$COLLECTION_NAME" ] || [ -z "$CODEBASE_PATH" ]; then
    if [ -f ".env" ]; then
        source ".env"
    fi
fi

# Validate required parameters
if [ -z "$COLLECTION_NAME" ]; then
    echo "Error: --collection required" >&2
    exit 1
fi

# Ensure the MCP server image is built
if ! docker image inspect vector-mcp-server:latest >/dev/null 2>&1; then
    echo "Building MCP server image..." >&2
    docker build -f Dockerfile.mcp -t vector-mcp-server:latest . >&2
fi

# Run the MCP server container with stdio passthrough
docker run --rm -i \
  --network host \
  -e CHROMA_HOST=localhost \
  -e CHROMA_PORT=8000 \
  -e COLLECTION_NAME="${COLLECTION_NAME}" \
  -e CODEBASE_PATH="${CODEBASE_PATH}" \
  vector-mcp-server:latest
