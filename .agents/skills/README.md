# Quasar agent skills

`.agents/skills` is the canonical location. It is read directly by Codex and
OpenCode. Claude Code expects `.claude/skills`, which is a symlink to this
directory, so all three agents see the same files.

Each skill is a directory containing `SKILL.md` with YAML frontmatter (`name`
and `description` are required). Optional `scripts/`, `references/`, and
`assets/` directories are loaded only when the skill needs them.

Keep skills portable: standard frontmatter, relative forward-slash paths, and
no agent-specific syntax in the body. If an agent needs an extra discovery
location, add a symlink rather than a copy.

## References

- Open Agent Skills specification: https://agentskills.io/specification
- Claude skill authoring best practices: https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices
- OpenAI Codex skill discovery: https://developers.openai.com/codex/skills
- OpenCode skill discovery: https://opencode.ai/docs/skills/

