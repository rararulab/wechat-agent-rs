# Commit Style — Conventional Commits (MANDATORY)

Every commit message MUST follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description> (#N)

<optional body>

Closes #N
```

- **Allowed types**: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`, `build`, `revert`
- **Scope** matches module or area: `feat(cli):`, `fix(config):`, `refactor(http):`
- **Breaking changes** use `!`: `feat(cli)!: change command args`
- Include `(#N)` issue reference in commit subject
- Include `Closes #N` in commit body
- Do NOT use free-form commit messages like `"update code"` or `"fix stuff"`
