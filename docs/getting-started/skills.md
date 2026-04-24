# Agent Skills

Agent Skills are a lightweight, open format for packaging task-specific workflows, references, and optional scripts. Each skill is a directory with a required `SKILL.md` file and optional `scripts/`, `references/`, and `assets/` folders. Agent Skills are designed for progressive disclosure: clients load the name and description first, then load the full instructions only when needed. The `allowed-tools` field is part of the spec but experimental and not supported by every client.

## Skills included in this repo

The repo now uses one standalone, persona-first skill package per workflow:

- `analyst`
- `bi-engineer`
- `engineer`
- `governance`
- `kpi-debugger`
- `meta-authoring`
- `model-architect`
- `project-cleanup`

Each skill now has:
- one canonical skill name
- one shared reasoning workflow
- transport selected inside the skill
- transport-specific references stored inside the same skill package

This is cleaner than the old `mcp-*` / `cli-*` split because the durable concept is the persona, not the transport.

The repo also ships deterministic helper scripts for architecture and cleanup workflows:
- `scripts/export_entity_inventory.py`
- `scripts/export_column_inventory.py`
- `scripts/build_overlap_report.py`
- `scripts/install_skills.sh`

These scripts call Nova CLI tools directly and keep their outputs aligned with the tool contracts rather than introducing a second reporting schema.

## Install in common tools

Use the standalone persona skill directly when you want one workflow.
Use `--all` when you want the full dbt-nova skill set.

### Installer shortcut (`~/.agents/skills`)

For one skill:

```bash
bash scripts/install_skills.sh --skill analyst --skills-dir "$HOME/.agents/skills"
bash scripts/install_skills.sh --skill engineer --skills-dir "$HOME/.agents/skills"
```

For all dbt-nova skills:

```bash
bash scripts/install_skills.sh --all --skills-dir "$HOME/.agents/skills"
```

This installs each skill under its standalone persona name and removes any previously installed
transport-prefixed compatibility copies for that same persona from the same destination.

### Codex (CLI and IDE)

Codex supports Agent Skills for both the CLI and IDE extensions. You can install skills per user or per repository:

- User scope: `~/.codex/skills/<skill-name>`
- Repo scope: `.codex/skills/<skill-name>`

**Recommended (repo scope): install the standalone persona-first skill directly.**

```bash
bash scripts/install_skills.sh --skill analyst --skills-dir .codex/skills
bash scripts/install_skills.sh --skill engineer --skills-dir .codex/skills
```

If you already use another generic persona skill in a user-level directory, prefer repo scope so this skill overrides it only for the current project.

Restart Codex after installing new skills. You can invoke a skill explicitly with `$skill-name` or let Codex select it automatically.

### Claude (Claude apps and Claude Code)

Anthropic supports Agent Skills across Claude apps, Claude Code, and the API. Skills are loaded automatically when relevant. In Claude apps, enable Skills in Settings > Capabilities (Team/Enterprise admins must enable them org-wide) and upload a ZIP file containing your skill folder. For Claude Code, you can install skills via plugins or create a `skills/` directory in your project or plugin root; for personal use you can also add skills to `~/.claude/skills`.

**Recommended (Claude apps):** install the standalone skill into a staging directory, then zip the installed skill directory and upload it.

```bash
tmp_skills_dir="$(mktemp -d)"
bash scripts/install_skills.sh --all --skills-dir "${tmp_skills_dir}"
cd "${tmp_skills_dir}"
zip -r analyst.skill.zip analyst
zip -r engineer.skill.zip engineer
```

**Recommended (Claude Code / personal): install the standalone skill directly.**

```bash
bash /path/to/repo/scripts/install_skills.sh --skill analyst --skills-dir "$HOME/.claude/skills"
bash /path/to/repo/scripts/install_skills.sh --skill engineer --skills-dir "$HOME/.claude/skills"
```

### Gemini CLI

Gemini CLI discovers skills from three tiers: workspace (`.gemini/skills/`), user (`~/.gemini/skills/`), and extensions. Workspace skills take precedence over user skills. You can manage skills using `/skills` commands or `gemini skills ...` from the terminal.

**Recommended (workspace scope): install the standalone skill directly.**

```bash
bash scripts/install_skills.sh --skill analyst --skills-dir .gemini/skills
bash scripts/install_skills.sh --skill engineer --skills-dir .gemini/skills
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
skills-ref validate .github/skills/analyst
skills-ref validate .github/skills/bi-engineer
skills-ref validate .github/skills/engineer
skills-ref validate .github/skills/governance
skills-ref validate .github/skills/kpi-debugger
skills-ref validate .github/skills/meta-authoring
skills-ref validate .github/skills/model-architect
skills-ref validate .github/skills/project-cleanup
```

## Further reading

- Agent Skills specification: https://agentskills.io/specification
- Codex skills docs: https://developers.openai.com/codex/skills
- Claude Skills help center: https://support.claude.com/en/articles/12512180-using-skills-in-claude
- Gemini CLI skills docs: https://geminicli.com/docs/cli/skills/
