# AGENTS.md

This project uses `CLAUDE.md` files as the canonical source of project-specific guidance.

## Core instruction

- Always read and strictly follow the latest content from `./CLAUDE.md` and any nested `CLAUDE.md` files in subdirectories.
- Treat `CLAUDE.md` as the single source of truth for all project guidelines, coding standards, architecture decisions, workflows, test/build commands, style preferences, and do-not rules.
- Use this `AGENTS.md` only as a lightweight pointer to `CLAUDE.md`; do not duplicate project-specific instructions here.

## When making code changes

- If a change requires updating project rules, conventions, commands, style guidelines, or any other persistent instructions, always update the relevant `CLAUDE.md` file instead of `AGENTS.md`.
- Never modify `AGENTS.md` to store project rules. Keep it minimal and focused on pointing to `CLAUDE.md`.
- After updating any `CLAUDE.md`, briefly mention what changed so it can be reviewed.

## Codex-specific note

- Follow sandbox and tool constraints from the active Codex environment, but defer to `CLAUDE.md` for all project-specific behavior.
