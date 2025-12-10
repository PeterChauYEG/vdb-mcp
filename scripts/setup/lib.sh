#!/bin/bash
# Shared library functions for setup scripts
# Source this file in other scripts: source scripts/setup/lib.sh

# ============================================================================
# Output Formatting
# ============================================================================

print_header() {
    echo ""
    echo "═══════════════════════════════════════════════════════════════"
    echo "  $1"
    echo "═══════════════════════════════════════════════════════════════"
    echo ""
}

print_step() {
    local step_num="$1"
    local step_name="$2"
    echo "[$step_num/5] $step_name"
    echo "────────────────────────────────────────────────────────────────"
}

print_success() {
    echo "✅ $1"
}

print_error() {
    echo "❌ Error: $1" >&2
}

print_warning() {
    echo "⚠️  Warning: $1"
}

print_info() {
    echo "   $1"
}

# ============================================================================
# Path Utilities
# ============================================================================

validate_codebase() {
    local path="$1"

    if [ ! -d "$path" ]; then
        print_error "Directory not found: $path"
        exit 1
    fi
}

get_absolute_path() {
    local path="$1"
    (cd "$path" && pwd)
}

get_collection_name() {
    local abs_path="$1"
    basename "$abs_path"
}

get_git_hash() {
    local abs_path="$1"

    if [ -d "$abs_path/.git" ]; then
        (cd "$abs_path" && git rev-parse HEAD 2>/dev/null || echo "")
    else
        echo ""
    fi
}

get_git_branch() {
    local abs_path="$1"

    if [ -d "$abs_path/.git" ]; then
        (cd "$abs_path" && git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")
    else
        echo ""
    fi
}

# ============================================================================
# Configuration Management
# ============================================================================

create_env_file() {
    local codebase_path="$1"
    local collection_name="$2"
    local git_hash="$3"
    local git_branch="$4"

    local project_root="$(cd "$SCRIPT_DIR" && pwd)"
    local env_file="$project_root/.env"

    cat > "$env_file" << EOF
# Vector MCP Configuration
# Generated: $(date)

# Path to codebase to index
CODEBASE_PATH=$codebase_path

# Collection name (auto-generated from directory name)
COLLECTION_NAME=$collection_name

# Git hash (for tracking index freshness)
GIT_HASH=$git_hash

# Git branch (for branch-based indexing)
GIT_BRANCH=$git_branch
EOF
}

# ============================================================================
# Indexing
# ============================================================================

start_indexing() {
    local project_root="$(cd "$SCRIPT_DIR" && pwd)"

    # Ensure data directory exists with proper permissions
    mkdir -p "$project_root/data/chroma"
    chmod 777 "$project_root/data/chroma" 2>/dev/null || true

    # Stop any existing containers first
    (cd "$project_root" && docker compose --profile index down > /dev/null 2>&1)

    # Start ChromaDB and embedding service first
    (cd "$project_root" && docker compose up -d chromadb > /dev/null 2>&1)
    (cd "$project_root" && docker compose --profile index up -d embedding > /dev/null 2>&1)

    # Wait for services to be ready
    print_info "Waiting for ChromaDB and embedding service to be ready..."
    sleep 15

    # Start indexer
    (cd "$project_root" && docker compose --profile index up -d indexer > /dev/null 2>&1)
    return $?
}

wait_for_indexing() {
    local project_root="$(cd "$SCRIPT_DIR" && pwd)"
    local timeout=600  # 10 minutes max
    local elapsed=0
    local check_interval=2

    print_info "Waiting for indexing to complete..."
    echo ""

    # Wait for indexer container to start or complete
    while [ $elapsed -lt 30 ]; do
        if docker ps -a --format "{{.Names}}" | grep -q "vector-mcp-indexer"; then
            break
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    if ! docker ps -a --format "{{.Names}}" | grep -q "vector-mcp-indexer"; then
        print_warning "Indexer container not found"
        return 1
    fi

    # Monitor logs for completion
    local last_line=""
    while [ $elapsed -lt $timeout ]; do
        # Check if container has exited
        if ! docker ps --format "{{.Names}}" | grep -q "vector-mcp-indexer"; then
            # Container exited - check if it was successful
            local exit_code=$(docker inspect vector-mcp-indexer --format='{{.State.ExitCode}}' 2>/dev/null || echo "1")
            local logs=$(docker logs vector-mcp-indexer 2>&1)

            if [ "$exit_code" = "0" ]; then
                echo ""

                # Check if it was already indexed (skipped)
                if echo "$logs" | grep -q "already indexed, skipping"; then
                    print_success "Already indexed - no changes detected"
                    local branch=$(echo "$logs" | grep "Git branch:" | sed 's/.*: //')
                    local commit=$(echo "$logs" | grep "Git commit:" | sed 's/.*: //')
                    local chunks=$(echo "$logs" | grep "Total chunks:" | sed 's/.*: //')
                    [ -n "$branch" ] && print_info "Branch: $branch"
                    [ -n "$commit" ] && print_info "Commit: $commit"
                    [ -n "$chunks" ] && print_info "Total chunks: $chunks"
                    return 0
                fi

                # Check for completion message
                if echo "$logs" | grep -qE "Indexing complete!|✅ Indexing complete"; then
                    print_success "Indexing complete!"

                    # Extract stats
                    local files=$(echo "$logs" | grep "Found .* total files" | sed -n 's/Found \([0-9]*\) total files/\1/p')
                    local chunks=$(echo "$logs" | grep "Total chunks:" | sed 's/.*: //')
                    local generated=$(echo "$logs" | grep "Generated .* chunks" | sed -n 's/Generated \([0-9]*\) chunks.*/\1/p')

                    [ -n "$files" ] && print_info "Files processed: $files"
                    [ -n "$generated" ] && print_info "Chunks generated: $generated"
                    [ -n "$chunks" ] && print_info "Total chunks in DB: $chunks"
                    return 0
                fi

                # Exit 0 but no recognized message - show summary from logs
                print_success "Indexer completed"
                local chunks=$(echo "$logs" | grep "Total chunks:" | sed 's/.*: //')
                [ -n "$chunks" ] && print_info "Total chunks: $chunks"
                return 0
            else
                print_error "Indexer failed with exit code: $exit_code"
                echo ""
                print_info "Last 20 lines of logs:"
                echo "$logs" | tail -20
                return 1
            fi
        fi

        # Get latest log line
        local current_line=$(docker logs vector-mcp-indexer 2>&1 | tail -1)

        # Show progress updates
        if [ "$current_line" != "$last_line" ]; then
            # Check for completion
            if echo "$current_line" | grep -qE "Indexing complete!|already indexed"; then
                sleep 2
                continue  # Let the exit handler deal with it
            fi

            # Show progress lines
            if echo "$current_line" | grep -qE "Found|Processed|Generated|Embedding|batch"; then
                echo -ne "\r\033[K   $current_line"
            fi

            last_line="$current_line"
        fi

        sleep $check_interval
        elapsed=$((elapsed + check_interval))
    done

    echo ""
    print_warning "Indexing timeout (${timeout}s) - still running in background"
    print_info "Check status: docker logs vector-mcp-indexer"
    return 1
}

cleanup_indexing_services() {
    local project_root="$(cd "$SCRIPT_DIR" && pwd)"

    # Stop embedding and indexer containers (keep chromadb running for MCP)
    (cd "$project_root" && docker compose --profile index stop embedding indexer > /dev/null 2>&1)
    (cd "$project_root" && docker compose --profile index rm -f embedding indexer > /dev/null 2>&1)
}

# ============================================================================
# Claude Code Configuration
# ============================================================================

configure_claude() {
    local codebase_path="$1"
    local collection_name="$2"
    local git_hash="$3"

    local project_root="$(cd "$SCRIPT_DIR" && pwd)"
    local mcp_config="$codebase_path/.mcp.json"
    local mcp_server_script="$project_root/mcp-server.sh"

    # Validate mcp-server.sh exists
    if [ ! -f "$mcp_server_script" ]; then
        print_error "mcp-server.sh not found at $mcp_server_script"
        return 1
    fi

    # Make executable
    chmod +x "$mcp_server_script"

    # Backup existing config
    if [ -f "$mcp_config" ]; then
        local backup="$mcp_config.backup-$(date +%Y%m%d-%H%M%S)"
        cp "$mcp_config" "$backup"
        print_info "Backed up existing config to $(basename "$backup")"
    fi

    # Create or update config
    if [ -f "$mcp_config" ]; then
        update_mcp_config "$mcp_config" "$mcp_server_script"
    else
        create_mcp_config "$mcp_config" "$mcp_server_script"
    fi

    return 0
}

create_mcp_config() {
    local config_path="$1"
    local script_path="$2"

    cat > "$config_path" << EOF
{
  "mcpServers": {
    "vector-search": {
      "type": "stdio",
      "command": "$script_path",
      "args": [],
      "env": {}
    }
  }
}
EOF
}

update_mcp_config() {
    local config_path="$1"
    local script_path="$2"

    # Try jq first, fall back to python3
    if command -v jq &> /dev/null; then
        jq --arg path "$script_path" \
           '.mcpServers["vector-search"] = {
               "type": "stdio",
               "command": $path,
               "args": [],
               "env": {}
           }' "$config_path" > "$config_path.tmp" && mv "$config_path.tmp" "$config_path"
    elif command -v python3 &> /dev/null; then
        python3 << PYEOF
import json

with open('$config_path', 'r') as f:
    config = json.load(f)

if 'mcpServers' not in config:
    config['mcpServers'] = {}

config['mcpServers']['vector-search'] = {
    'type': 'stdio',
    'command': '$script_path',
    'args': [],
    'env': {}
}

with open('$config_path', 'w') as f:
    json.dump(config, f, indent=2)
PYEOF
    else
        print_error "Need either 'jq' or 'python3' to update existing config"
        print_info "Please install one or manually edit $config_path"
        return 1
    fi
}

# ============================================================================
# Git Hooks
# ============================================================================

# Hook version - increment when hook content changes
HOOK_VERSION="3"

hooks_need_update() {
    local codebase_path="$1"

    for hook_name in post-commit post-merge post-checkout; do
        local hook_path="$codebase_path/.git/hooks/$hook_name"

        # No hook exists
        if [ ! -f "$hook_path" ]; then
            return 0
        fi

        # Check if it's our hook and if version matches
        if grep -q "Auto-generated by vector-mcp" "$hook_path" 2>/dev/null; then
            local current_version=$(grep "HOOK_VERSION=" "$hook_path" 2>/dev/null | sed 's/.*HOOK_VERSION=//' | tr -d '"')
            if [ "$current_version" != "$HOOK_VERSION" ]; then
                return 0
            fi
        else
            # Not our hook - needs handling
            return 0
        fi
    done

    return 1
}

install_git_hooks() {
    local codebase_path="$1"
    local project_root="$(cd "$SCRIPT_DIR" && pwd)"

    local hooks_installed=0

    for hook_name in post-commit post-merge post-checkout; do
        local hook_path="$codebase_path/.git/hooks/$hook_name"

        # Handle existing hooks
        if [ -f "$hook_path" ]; then
            if handle_existing_hook "$hook_path" "$hook_name" "$project_root"; then
                hooks_installed=$((hooks_installed + 1))
            fi
        else
            # Create new hook
            create_git_hook "$hook_path" "$project_root"
            hooks_installed=$((hooks_installed + 1))
        fi
    done

    if [ $hooks_installed -gt 0 ]; then
        print_info "Installed $hooks_installed git hooks"
        print_info "• post-commit: Re-indexes after commits"
        print_info "• post-merge: Re-indexes after git pull"
        print_info "• post-checkout: Re-indexes after branch switch"
        return 0
    else
        return 1
    fi
}

handle_existing_hook() {
    local hook_path="$1"
    local hook_name="$2"
    local project_root="$3"

    # Check if already has vector-mcp
    if grep -q "Auto-generated by vector-mcp" "$hook_path" 2>/dev/null; then
        # Check version
        local current_version=$(grep "HOOK_VERSION=" "$hook_path" 2>/dev/null | sed 's/.*HOOK_VERSION=//' | tr -d '"')

        if [ "$current_version" = "$HOOK_VERSION" ]; then
            # Already up to date
            return 1
        fi

        # Check if it's pure vector-mcp hook (safe to overwrite)
        local total_lines=$(wc -l < "$hook_path")
        local vmp_lines=$(grep -c "vector-mcp\|MCP_ROOT\|Re-indexing\|HOOK_VERSION" "$hook_path" 2>/dev/null || echo 0)

        if [ "$vmp_lines" -gt $((total_lines - 5)) ]; then
            # Pure vector-mcp hook - update it
            create_git_hook "$hook_path" "$project_root"
            print_info "Updated $hook_name hook"
            return 0
        else
            # Mixed hook - keep it but warn
            print_info "$hook_name has custom code, skipping update"
            return 1
        fi
    else
        # Non-vector-mcp hook found
        echo ""
        echo "⚠️  Existing $hook_name hook detected"
        read -p "Backup and append vector-mcp code? (y/N) " -n 1 -r
        echo ""

        if [[ $REPLY =~ ^[Yy]$ ]]; then
            local backup="$hook_path.backup-$(date +%Y%m%d-%H%M%S)"
            cp "$hook_path" "$backup"
            print_info "Backed up to: $(basename "$backup")"

            # Append to existing
            cat >> "$hook_path" << EOHOOK

# ========================================
# Auto-generated by vector-mcp
# HOOK_VERSION=$HOOK_VERSION
# Added: $(date)
# ========================================

MCP_ROOT="$project_root"
if [ -d "\$MCP_ROOT" ]; then
    echo "🔄 Vector MCP: Re-indexing..."
    # Update git hash/branch in .env before starting indexer
    NEW_HASH=\$(git rev-parse HEAD)
    NEW_BRANCH=\$(git rev-parse --abbrev-ref HEAD)
    if [ -f "\$MCP_ROOT/.env" ]; then
        sed -i.bak "s/^GIT_HASH=.*/GIT_HASH=\$NEW_HASH/" "\$MCP_ROOT/.env"
        sed -i.bak "s/^GIT_BRANCH=.*/GIT_BRANCH=\$NEW_BRANCH/" "\$MCP_ROOT/.env"
        rm -f "\$MCP_ROOT/.env.bak"
    fi
    (cd "\$MCP_ROOT" && docker compose up -d chromadb && docker compose --profile index up -d embedding indexer > /dev/null 2>&1 &)
fi
EOHOOK

            print_info "Appended to $hook_name"
            return 0
        else
            print_info "Skipped $hook_name"
            return 1
        fi
    fi
}

create_git_hook() {
    local hook_path="$1"
    local project_root="$2"

    cat > "$hook_path" << EOF
#!/bin/bash
# Auto-generated by vector-mcp - Re-index after git operations
# HOOK_VERSION=$HOOK_VERSION
# This file is git-ignored and local to your machine

MCP_ROOT="$project_root"

echo "🔄 Vector MCP: Re-indexing codebase..."

# Update git hash/branch in .env before starting indexer
NEW_HASH=\$(git rev-parse HEAD)
NEW_BRANCH=\$(git rev-parse --abbrev-ref HEAD)
if [ -f "\$MCP_ROOT/.env" ]; then
    sed -i.bak "s/^GIT_HASH=.*/GIT_HASH=\$NEW_HASH/" "\$MCP_ROOT/.env"
    sed -i.bak "s/^GIT_BRANCH=.*/GIT_BRANCH=\$NEW_BRANCH/" "\$MCP_ROOT/.env"
    rm -f "\$MCP_ROOT/.env.bak"
fi

(cd "\$MCP_ROOT" && docker compose up -d chromadb && docker compose --profile index up -d embedding indexer > /dev/null 2>&1 &)
EOF

    chmod +x "$hook_path"
}
