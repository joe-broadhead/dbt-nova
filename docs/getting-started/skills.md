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
  - `mcp-nova-meta-authoring`
  - `mcp-project-cleanup`
- `.github/skills/cli/`
  - `cli-analyst`
  - `cli-bi-engineer`
  - `cli-engineer`
  - `cli-governance`
  - `cli-kpi-debugger`
  - `cli-model-architect`
  - `cli-nova-meta-authoring`
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

## Install in common tools

### Installer shortcut (`~/.agents/skills`)

If you installed dbt-nova via `scripts/install.sh`, you can also install skills into
the standard Agent Skills user directory in one step:

```bash
curl -fsSL https://raw.githubusercontent.com/joe-broadhead/dbt-nova/master/scripts/install.sh | \
  bash -s -- --slim --install-skills --non-interactive
```

Set `DBT_NOVA_SKILLS_DIR` to use a different destination (for example
`~/.codex/skills` or `~/.claude/skills`).
The installer flattens bundle paths into unique skill names such as
`mcp-analyst` and `cli-analyst`.

### Codex (CLI and IDE)

Codex supports Agent Skills for both the CLI and IDE extensions. You can install skills per user or per repository:

- User scope: `~/.codex/skills/<skill-name>`
- Repo scope: `.codex/skills/<skill-name>`

**Recommended (repo scope):**

```bash
mkdir -p .codex/skills
cp -R .github/skills/mcp/analyst .codex/skills/mcp-analyst
cp -R .github/skills/cli/analyst .codex/skills/cli-analyst
cp -R .github/skills/mcp/bi-engineer .codex/skills/mcp-bi-engineer
cp -R .github/skills/cli/bi-engineer .codex/skills/cli-bi-engineer
cp -R .github/skills/mcp/engineer .codex/skills/mcp-engineer
cp -R .github/skills/cli/engineer .codex/skills/cli-engineer
cp -R .github/skills/mcp/governance .codex/skills/mcp-governance
cp -R .github/skills/cli/governance .codex/skills/cli-governance
cp -R .github/skills/mcp/kpi-debugger .codex/skills/mcp-kpi-debugger
cp -R .github/skills/cli/kpi-debugger .codex/skills/cli-kpi-debugger
cp -R .github/skills/mcp/model-architect .codex/skills/mcp-model-architect
cp -R .github/skills/cli/model-architect .codex/skills/cli-model-architect
cp -R .github/skills/mcp/nova-meta-authoring .codex/skills/mcp-nova-meta-authoring
cp -R .github/skills/cli/nova-meta-authoring .codex/skills/cli-nova-meta-authoring
cp -R .github/skills/mcp/project-cleanup .codex/skills/mcp-project-cleanup
cp -R .github/skills/cli/project-cleanup .codex/skills/cli-project-cleanup
```

Restart Codex after installing new skills. You can invoke a skill explicitly with `$skill-name` or let Codex select it automatically.

### Claude (Claude apps and Claude Code)

Anthropic supports Agent Skills across Claude apps, Claude Code, and the API. Skills are loaded automatically when relevant. In Claude apps, enable Skills in Settings > Capabilities (Team/Enterprise admins must enable them org-wide) and upload a ZIP file containing your skill folder. For Claude Code, you can install skills via plugins or create a `skills/` directory in your project or plugin root; for personal use you can also add skills to `~/.claude/skills`.

**Recommended (Claude apps):** zip a single skill directory and upload it.

```bash
cd .github/skills/mcp
zip -r mcp-analyst.skill.zip analyst
```

**Recommended (Claude Code / personal):**

```bash
mkdir -p ~/.claude/skills
cp -R /path/to/repo/.github/skills/mcp/analyst ~/.claude/skills/mcp-analyst
cp -R /path/to/repo/.github/skills/cli/analyst ~/.claude/skills/cli-analyst
cp -R /path/to/repo/.github/skills/mcp/bi-engineer ~/.claude/skills/mcp-bi-engineer
cp -R /path/to/repo/.github/skills/cli/bi-engineer ~/.claude/skills/cli-bi-engineer
cp -R /path/to/repo/.github/skills/mcp/engineer ~/.claude/skills/mcp-engineer
cp -R /path/to/repo/.github/skills/cli/engineer ~/.claude/skills/cli-engineer
cp -R /path/to/repo/.github/skills/mcp/governance ~/.claude/skills/mcp-governance
cp -R /path/to/repo/.github/skills/cli/governance ~/.claude/skills/cli-governance
cp -R /path/to/repo/.github/skills/mcp/kpi-debugger ~/.claude/skills/mcp-kpi-debugger
cp -R /path/to/repo/.github/skills/cli/kpi-debugger ~/.claude/skills/cli-kpi-debugger
cp -R /path/to/repo/.github/skills/mcp/model-architect ~/.claude/skills/mcp-model-architect
cp -R /path/to/repo/.github/skills/cli/model-architect ~/.claude/skills/cli-model-architect
cp -R /path/to/repo/.github/skills/mcp/nova-meta-authoring ~/.claude/skills/mcp-nova-meta-authoring
cp -R /path/to/repo/.github/skills/cli/nova-meta-authoring ~/.claude/skills/cli-nova-meta-authoring
cp -R /path/to/repo/.github/skills/mcp/project-cleanup ~/.claude/skills/mcp-project-cleanup
cp -R /path/to/repo/.github/skills/cli/project-cleanup ~/.claude/skills/cli-project-cleanup
```

### Gemini CLI

Gemini CLI discovers skills from three tiers: workspace (`.gemini/skills/`), user (`~/.gemini/skills/`), and extensions. Workspace skills take precedence over user skills. You can manage skills using `/skills` commands or `gemini skills ...` from the terminal.

**Recommended (workspace scope):**

```bash
mkdir -p .gemini/skills
cp -R .github/skills/mcp/analyst .gemini/skills/mcp-analyst
cp -R .github/skills/cli/analyst .gemini/skills/cli-analyst
cp -R .github/skills/mcp/bi-engineer .gemini/skills/mcp-bi-engineer
cp -R .github/skills/cli/bi-engineer .gemini/skills/cli-bi-engineer
cp -R .github/skills/mcp/engineer .gemini/skills/mcp-engineer
cp -R .github/skills/cli/engineer .gemini/skills/cli-engineer
cp -R .github/skills/mcp/governance .gemini/skills/mcp-governance
cp -R .github/skills/cli/governance .gemini/skills/cli-governance
cp -R .github/skills/mcp/kpi-debugger .gemini/skills/mcp-kpi-debugger
cp -R .github/skills/cli/kpi-debugger .gemini/skills/cli-kpi-debugger
cp -R .github/skills/mcp/model-architect .gemini/skills/mcp-model-architect
cp -R .github/skills/cli/model-architect .gemini/skills/cli-model-architect
cp -R .github/skills/mcp/nova-meta-authoring .gemini/skills/mcp-nova-meta-authoring
cp -R .github/skills/cli/nova-meta-authoring .gemini/skills/cli-nova-meta-authoring
cp -R .github/skills/mcp/project-cleanup .gemini/skills/mcp-project-cleanup
cp -R .github/skills/cli/project-cleanup .gemini/skills/cli-project-cleanup
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
skills-ref validate .github/skills/cli/nova-meta-authoring
```

## Further reading

- Agent Skills specification: https://agentskills.io/specification
- Codex skills docs: https://developers.openai.com/codex/skills
- Claude Skills help center: https://support.claude.com/en/articles/12512180-using-skills-in-claude
- Gemini CLI skills docs: https://geminicli.com/docs/cli/skills/
