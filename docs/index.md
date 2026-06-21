---
hide:
  - navigation
  - toc
---

# dbt-nova

<p class="subtitle">
High-performance MCP server for dbt manifest search and analysis.
</p>

DBT Nova bridges raw dbt artifacts to agent-ready intelligence by normalizing
manifest and `meta.nova` metadata into indexed, queryable representations and
exposing them through MCP tools for discovery, lineage, scoring, and governance.

<div class="grid cards" markdown>

-   :material-clock-fast:{ .lg .middle } __Get Started in 5 Minutes__

    ---

    Install dbt-nova and connect it to your favorite AI assistant

    [:octicons-arrow-right-24: Quick Start](getting-started/quickstart.md)

-   :material-magnify:{ .lg .middle } __Powerful Search__

    ---

    Hybrid search with BM25, vector embeddings, and ML reranking

    [:octicons-arrow-right-24: Search Features](features/search-ranking.md)

-   :material-star-check:{ .lg .middle } __Metadata Scoring__

    ---

    Automated quality scoring for documentation, governance, and more

    [:octicons-arrow-right-24: Metadata Scoring](features/metadata-scoring.md)

-   :material-database-search:{ .lg .middle } __Nova Meta 101__

    ---

    The schema and field map that power search, scoring, and governance

    [:octicons-arrow-right-24: Nova Meta Overview](features/nova-meta-overview.md)

-   :material-account-group:{ .lg .middle } __Persona Workflows__

    ---

    Tailored experiences for analysts, engineers, and governance teams

    [:octicons-arrow-right-24: Personas](personas/overview.md)

-   :material-playlist-check:{ .lg .middle } __Analysis Recipes__

    ---

    Deterministic, reusable SQL workflows for recurring reporting topics

    [:octicons-arrow-right-24: Recipe Guide](features/recipes.md)

</div>

## Which Persona Are You?

| If you need to... | You're a... | Start here |
|-------------------|-------------|------------|
| Find datasets, validate metrics, create reports | **Analyst** | [Analyst Guide](personas/analyst.md) |
| Debug models, assess impact, fix tests | **Engineer** | [Engineer Guide](personas/engineer.md) |
| Audit compliance, find gaps, track ownership | **Governance** | [Governance Guide](personas/governance.md) |

## Quick Install

```bash
# Download and install (supported prebuilt release targets)
# Linux (x86_64, Cloud Run compatible)
gh release download --repo joe-broadhead/dbt-nova \
  --pattern dbt-nova-linux-x86_64.tar.gz --output dbt-nova-linux-x86_64.tar.gz
tar -xzf dbt-nova-linux-x86_64.tar.gz
sudo mv dbt-nova /usr/local/bin/

# macOS (Apple Silicon)
# gh release download --repo joe-broadhead/dbt-nova \
#   --pattern dbt-nova-macos-arm64.tar.gz --output dbt-nova-macos-arm64.tar.gz
# tar -xzf dbt-nova-macos-arm64.tar.gz
# sudo mv dbt-nova /usr/local/bin/

# Other platforms: build from source
# git clone https://github.com/joe-broadhead/dbt-nova.git
# cd dbt-nova && cargo build --release

# Set manifest path
export DBT_MANIFEST_PATH=/path/to/manifest.json

# Run server
dbt-nova
```

## Features

| Feature | Description |
|---------|-------------|
| **35 MCP Tools** | Server-exposed MCP surface for search, lineage, coverage, scoring, SQL execution, reports, and recipes |
| **20 CLI Leaf Commands** | One-shot commands for server startup, manifest lifecycle, audits, config, storage, evals, health, and tool calls |
| **Hybrid Search** | Tantivy BM25 + vector + sparse + reranking |
| **Multi-Source** | Local files, HTTP, S3, GCS, DBFS |
| **Personas** | Analyst, Engineer, Governance profiles |
| **Production Ready** | Rate limiting, caching, circuit breakers |

## Nova Meta 101

Nova meta is the foundation of high‑signal search, scoring, and governance.
If you only read one guide, start here:

[:octicons-arrow-right-24: Nova Meta Overview](features/nova-meta-overview.md)

## Documentation

<div class="grid cards" markdown>

-   :material-book-open-variant:{ .lg .middle } __Configuration__

    ---

    All configuration options and environment variables

    [:octicons-arrow-right-24: Configuration](configuration/reference.md)

-   :material-tools:{ .lg .middle } __Tools Reference__

    ---

    Complete reference for all MCP tools

    [:octicons-arrow-right-24: Tools](api/tools.md)

-   :material-source-branch:{ .lg .middle } __Architecture__

    ---

    System design, data flow, and internals

    [:octicons-arrow-right-24: Architecture](development/architecture.md)

-   :material-lifebuoy:{ .lg .middle } __Operations__

    ---

    Troubleshooting, monitoring, and maintenance

    [:octicons-arrow-right-24: Operations](operations/ops.md)

</div>
