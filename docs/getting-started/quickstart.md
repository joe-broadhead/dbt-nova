# Quick Start

Get dbt-nova running in under 5 minutes.

## Prerequisites

- A dbt project with a compiled `manifest.json`
- An MCP-compatible AI client (Claude Desktop, Cursor, etc.)

## Step 1: Install

!!! tip "All-in-one option"
    Use the installer script. It defaults to the **slim** release and
    starts with lexical search only. Semantic layers are opt-in.

```bash
# Public repo (unauthenticated)
curl -fsSL https://raw.githubusercontent.com/joe-broadhead/dbt-nova/master/scripts/install.sh | \
  bash -s -- --slim --non-interactive

# Private repo (authenticated)
GH_TOKEN="$(gh auth token)"
curl -fsSL -H "Authorization: Bearer ${GH_TOKEN}" \
  https://raw.githubusercontent.com/joe-broadhead/dbt-nova/master/scripts/install.sh | \
  DBT_NOVA_GITHUB_TOKEN="${GH_TOKEN}" bash -s -- --slim --non-interactive

# Optional: pre-warm models during install before enabling semantic layers
curl -fsSL https://raw.githubusercontent.com/joe-broadhead/dbt-nova/master/scripts/install.sh | \
  DBT_NOVA_EMBEDDINGS_CACHE_DIR="$HOME/.dbt-nova/.fastembed_cache" \
  DBT_NOVA_WARMUP_REQUIRED_MODELS=3 \
  bash -s -- --slim --warm-models --non-interactive

# Optional: install Agent Skills into ~/.agents/skills
curl -fsSL https://raw.githubusercontent.com/joe-broadhead/dbt-nova/master/scripts/install.sh | \
  bash -s -- --slim --install-skills --non-interactive
```

=== "macOS (Apple Silicon)"

    ```bash
    gh release download --repo joe-broadhead/dbt-nova \
      --pattern dbt-nova-macos-arm64.tar.gz --output dbt-nova-macos-arm64.tar.gz
    tar -xzf dbt-nova-macos-arm64.tar.gz
    sudo mv dbt-nova /usr/local/bin/
    ```

=== "Linux (x64)"

    ```bash
    gh release download --repo joe-broadhead/dbt-nova \
      --pattern dbt-nova-linux-x86_64.tar.gz --output dbt-nova-linux-x86_64.tar.gz
    tar -xzf dbt-nova-linux-x86_64.tar.gz
    sudo mv dbt-nova /usr/local/bin/
    ```

=== "From Source"

    ```bash
    git clone https://github.com/joe-broadhead/dbt-nova.git
    cd dbt-nova
    cargo build --release
    # default mode is partial (model files only); falls back if direct download fails
    bash scripts/warm_models.sh
    # optional integrity enforcement for direct HF fallback:
    # DBT_NOVA_WARMUP_CHECKSUM_MODE=required DBT_NOVA_WARMUP_CHECKSUM_FILE=/path/to/checksums.txt bash scripts/warm_models.sh
    # optional full mode: model files + embedding caches for a specific manifest
    # DBT_NOVA_WARMUP_MANIFEST_PATH=/path/to/manifest.json bash scripts/warm_models.sh full
    sudo cp target/release/dbt-nova /usr/local/bin/
    ```

## Step 2: Configure

Set your manifest path:

```bash
export PATH="$HOME/.local/bin:$PATH"
export DBT_MANIFEST_PATH=/path/to/your/dbt/project/target/manifest.json
export DBT_NOVA_EMBEDDINGS_CACHE_DIR="$HOME/.dbt-nova/.fastembed_cache"

# Optional: enable semantic search after warming models/caches
# export DBT_NOVA_SEARCH_ENABLE_VECTOR=true
# export DBT_NOVA_SEARCH_ENABLE_SPARSE=true
# export DBT_NOVA_SEARCH_ENABLE_RERANKER=true
```

Use the same `DBT_NOVA_EMBEDDINGS_CACHE_DIR` value in your MCP client config.

!!! tip "Pro Tip"
    Add this to your shell profile (`~/.bashrc`, `~/.zshrc`) to persist across sessions.

### Optional: Consume Prebuilt Artifacts

If you build assets in CI with the reusable workflow, consumers can start
without manual extraction by setting:

- `DBT_NOVA_BOOTSTRAP_URI`
- `DBT_NOVA_ARTIFACT_FETCH_POLICY=if_missing`
- `DBT_NOVA_STORAGE_READ_ONLY` unset or `false` on first run
- `DBT_NOVA_STORAGE_DIR`

Strict read-only mode is only for reuse after artifacts already exist locally:

- `DBT_NOVA_STORAGE_READ_ONLY=true`
- `DBT_NOVA_ARTIFACT_FETCH_POLICY=never`

Optional explicit mode:

- `DBT_NOVA_STORAGE_INSTANCE_ID`
- `DBT_NOVA_STORAGE_ARTIFACT_URI`
- `DBT_NOVA_METADATA_ARTIFACT_URI`
- optional `DBT_NOVA_MODELS_ARTIFACT_URI`

See [Prebuilt Asset Workflow](../operations/prebuilt-assets.md) for full producer/consumer setup.

### Optional: Remote Manifests

dbt-nova supports loading manifests from remote sources:

```bash
# From S3
export DBT_NOVA_MANIFEST_URI=s3://my-bucket/manifest.json

# From GCS
export DBT_NOVA_MANIFEST_URI=gs://my-bucket/manifest.json

# From HTTP
export DBT_NOVA_MANIFEST_URI=https://example.com/manifest.json

# From Databricks DBFS
export DBT_NOVA_MANIFEST_URI=dbfs:///path/to/manifest.json
```

See [Manifest Sources](../configuration/manifest-sources.md) for authentication details.

## Step 3: Connect to MCP Client

### Claude Desktop

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "dbt-nova": {
      "command": "dbt-nova",
      "env": {
        "DBT_MANIFEST_PATH": "/path/to/manifest.json",
        "DBT_NOVA_EMBEDDINGS_CACHE_DIR": "/Users/<you>/.dbt-nova/.fastembed_cache"
      }
    }
  }
}
```

### Cursor

Add to `.cursor/mcp.json` in your project:

```json
{
  "mcpServers": {
    "dbt-nova": {
      "command": "dbt-nova",
      "env": {
        "DBT_MANIFEST_PATH": "/path/to/manifest.json",
        "DBT_NOVA_EMBEDDINGS_CACHE_DIR": "/Users/<you>/.dbt-nova/.fastembed_cache"
      }
    }
  }
}
```

See [MCP Clients](mcp-clients.md) for more client configurations.

## Step 4: Verify

Test the server directly:

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | dbt-nova
```

You should see a JSON response listing all available tools.

!!! success "You're Ready!"
    Once connected to your MCP client, you can ask questions like:

    - "Search for models related to customers"
    - "Show me the lineage of the orders model"
    - "What's the test coverage for staging models?"
    - "Score the metadata quality of my project"

## Step 5: Run Your First Recipe

For recurring workflows (weekly KPI packs, MBRs, channel reports), start with
recipes instead of ad-hoc SQL.

### 1) Discover a recipe

```json
{
  "name": "search_recipes",
  "arguments": {
    "topic": "weekly",
    "query": "digital country",
    "include_queries": true,
    "limit": 5
  }
}
```

### 2) Inspect the recipe contract

```json
{
  "name": "get_recipe",
  "arguments": {
    "recipe_id": "weekly_country_kpi_report",
    "include_queries": true
  }
}
```

### 3) Execute deterministically

```json
{
  "name": "run_recipe",
  "arguments": {
    "recipe_id": "weekly_country_kpi_report",
    "parameters": {
      "COUNTRY_CODE": "GB",
      "WEEK_START": "2026-02-01",
      "WEEK_END": "2026-02-07"
    },
    "stop_on_failure": true
  }
}
```

See [Analysis Recipes](../features/recipes.md) for naming conventions,
parameter contracts, and execution order rules.

## Next Steps

- [CLI Commands](cli.md) - One-shot command groups, JSON mode, and exit codes
- [Modes & Combinations](modes-and-combinations.md) - Install/runtime decision matrix
- [Configuration Reference](../configuration/reference.md) - All configuration options
- [Tools Reference](../api/tools.md) - Available tools and parameters
- [Personas](../personas/overview.md) - Workflow guides by role
- [Analysis Recipes](../features/recipes.md) - Deterministic recurring workflows
- [Metadata Scoring](../features/metadata-scoring.md) - Understand quality scores
