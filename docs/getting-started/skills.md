# Agent Skills

Agent Skills are a lightweight, open format for packaging task-specific workflows, references, and optional scripts. Each skill is a directory with a required `SKILL.md` file and optional `scripts/`, `references/`, and `assets/` folders. Agent Skills are designed for progressive disclosure: clients load the name and description first, then load the full instructions only when needed. The `allowed-tools` field is part of the spec but experimental and not supported by every client.

## Skills included in this repo

We ship two built-in skill bundles under `.github/skills/`:

- `.github/skills/mcp/`
  - `mcp-analyst`
  - `mcp-bi-engineer`
  - `mcp-engineer`
  - `mcp-governance`
  - `mcp-kpi-debugger`
  - `mcp-model-architect`
  - `mcp-meta-authoring`
  - `mcp-project-cleanup`
- `.github/skills/cli/`
  - `cli-analyst`
  - `cli-bi-engineer`
  - `cli-engineer`
  - `cli-governance`
  - `cli-kpi-debugger`
  - `cli-model-architect`
  - `cli-meta-authoring`
  - `cli-project-cleanup`
- `.github/skills/shared/`
  - transport-agnostic references and assets used by both bundles

Use the `mcp-*` skills when the agent can call Nova MCP tools directly.
Use the `cli-*` skills when the agent only has terminal access to `dbt-nova` commands such as
`tool call`, `manifest load`, `health check`, `audit metadata-score`, and `audit nova-meta`.

Each installable skill follows the Agent Skills spec. The repo structure now separates:
- thin transport wrappers in `.github/skills/cli/` and `.github/skills/mcp/`
- shared workflow references and reusable assets in `.github/skills/shared/`

This keeps the real workflow logic in one place while letting each transport wrapper stay focused on:
- session setup
- transport-specific caveats
- command/tool syntax
- boundaries where CLI and MCP differ

The shared layer also holds reusable output templates for current and upcoming skills, including:
- analyst evidence and report templates
- engineer ship checklists
- governance audit and remediation queue templates
- BI engineer dashboard, metric card, dataset contract, and viz QA templates
- KPI investigation, refactor-plan, and overlap-audit templates for future debugger/architecture/cleanup skills

The repo also ships deterministic helper scripts for architecture and cleanup workflows:
- `scripts/export_entity_inventory.py`
- `scripts/export_column_inventory.py`
- `scripts/build_overlap_report.py`
- `scripts/install_skills.sh`

These scripts call Nova CLI tools directly and keep their outputs aligned with the tool contracts rather than introducing a second reporting schema.

## Install in common tools

Use one bundle per client scope. If you want both MCP-first and terminal-first workflows,
install them into different clients or different skill directories rather than mixing both
bundles into the same destination.

### Installer shortcut (`~/.agents/skills`)

If you installed dbt-nova via `scripts/install.sh`, you can also install skills into
the standard Agent Skills user directory in one step. Choose exactly one bundle:

- `mcp`: for agents that call Nova MCP tools directly
- `cli`: for agents that only use terminal access to `dbt-nova`

```bash
curl -fsSL https://raw.githubusercontent.com/joe-broadhead/dbt-nova/master/scripts/install.sh | \
  bash -s -- --slim --install-skills --skills-bundle mcp --non-interactive
```

Set `DBT_NOVA_SKILLS_DIR` to use a different destination (for example
`~/.codex/skills` or `~/.claude/skills`).
Set `DBT_NOVA_SKILLS_BUNDLE=cli` or pass `--skills-bundle cli` to install the CLI
bundle instead. The installer flattens bundle paths into unique skill names such as
`mcp-analyst` and removes any previously installed dbt-nova skills from the other
bundle in that same destination.

### Codex (CLI and IDE)

Codex supports Agent Skills for both the CLI and IDE extensions. You can install skills per user or per repository:

- User scope: `~/.codex/skills/<skill-name>`
- Repo scope: `.codex/skills/<skill-name>`

**Recommended (repo scope): choose one bundle and install standalone skills from the local checkout.**

```bash
bash scripts/install_skills.sh --bundle mcp --skills-dir .codex/skills
```

Use `--bundle cli` instead when the agent should rely on terminal access rather than MCP.
`install_skills.sh` copies the shared references and assets into each installed skill
so the result is standalone and client-safe.

Restart Codex after installing new skills. You can invoke a skill explicitly with `$skill-name` or let Codex select it automatically.

### Claude (Claude apps and Claude Code)

Anthropic supports Agent Skills across Claude apps, Claude Code, and the API. Skills are loaded automatically when relevant. In Claude apps, enable Skills in Settings > Capabilities (Team/Enterprise admins must enable them org-wide) and upload a ZIP file containing your skill folder. For Claude Code, you can install skills via plugins or create a `skills/` directory in your project or plugin root; for personal use you can also add skills to `~/.claude/skills`.

**Recommended (Claude apps):** install one bundle into a staging directory, then zip a single installed skill directory and upload it.

```bash
tmp_skills_dir="$(mktemp -d)"
bash scripts/install_skills.sh --bundle mcp --skills-dir "${tmp_skills_dir}"
cd "${tmp_skills_dir}"
zip -r mcp-analyst.skill.zip mcp-analyst
```

**Recommended (Claude Code / personal): choose one bundle.**

```bash
bash /path/to/repo/scripts/install_skills.sh --bundle mcp --skills-dir "$HOME/.claude/skills"
```

### Gemini CLI

Gemini CLI discovers skills from three tiers: workspace (`.gemini/skills/`), user (`~/.gemini/skills/`), and extensions. Workspace skills take precedence over user skills. You can manage skills using `/skills` commands or `gemini skills ...` from the terminal.

**Recommended (workspace scope): choose one bundle.**

```bash
bash scripts/install_skills.sh --bundle mcp --skills-dir .gemini/skills
```

Then reload skills:

```bash
gemini skills list
gemini skills reload
```

Gemini CLI can also install skills directly from a repo or local path:

```bash
gemini skills install /path/to/skill --scope workspace
```

## Validation (optional)

Use the reference tooling from the Agent Skills project to validate your skills:

```bash
skills-ref validate .github/skills/mcp/analyst
skills-ref validate .github/skills/cli/meta-authoring
```

## Further reading

- Agent Skills specification: https://agentskills.io/specification
- Codex skills docs: https://developers.openai.com/codex/skills
- Claude Skills help center: https://support.claude.com/en/articles/12512180-using-skills-in-claude
- Gemini CLI skills docs: https://geminicli.com/docs/cli/skills/
