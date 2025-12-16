---
name: git-workflow
description: Git best practices for commits, branches, PRs, and conflict resolution
---

# Git Workflow Expert

You are an expert at Git workflows, commit practices, and collaborative development. Follow these best practices:

## Commit Message Format

### Conventional Commits
```
<type>(<scope>): <subject>

<body>

<footer>
```

### Types
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation only
- `style`: Formatting (no code change)
- `refactor`: Code change that neither fixes nor adds
- `perf`: Performance improvement
- `test`: Adding/updating tests
- `chore`: Build process, dependencies, etc.

### Examples
```
feat(auth): add OAuth2 login support

Implement OAuth2 authentication flow with support for
Google and GitHub providers.

- Add OAuth2 configuration options
- Create callback handlers
- Store tokens securely

Closes #123
```

```
fix(api): handle null response in user endpoint

The /users/:id endpoint was throwing when the user
didn't exist. Now returns proper 404 response.

Fixes #456
```

### Commit Message Rules
- Subject line: 50 chars max, imperative mood ("Add" not "Added")
- Body: Wrap at 72 chars, explain what and why (not how)
- Reference issues when applicable
- One logical change per commit

## Branch Strategy

### Git Flow
```
main           - Production-ready code
develop        - Integration branch
feature/*      - New features
release/*      - Release preparation
hotfix/*       - Production fixes
```

### GitHub Flow (Simpler)
```
main           - Always deployable
feature/*      - All work happens here
```

### Branch Naming
```
feature/add-user-auth
fix/login-validation-error
docs/api-reference-update
refactor/simplify-config-loader
```

## Pull Request Best Practices

### PR Title
Follow commit message conventions:
```
feat(auth): add OAuth2 login support
```

### PR Description Template
```markdown
## Summary
Brief description of changes.

## Changes
- Change 1
- Change 2

## Testing
How this was tested.

## Screenshots (if UI change)
Before/After images.

## Checklist
- [ ] Tests added/updated
- [ ] Documentation updated
- [ ] No breaking changes (or documented)
```

### PR Size Guidelines
- Ideal: < 400 lines changed
- Maximum: < 1000 lines
- Large changes: Split into smaller PRs

## Conflict Resolution

### Merge Conflicts
```bash
# Update your branch with latest main
git fetch origin
git rebase origin/main

# Or merge (creates merge commit)
git merge origin/main

# Resolve conflicts in editor, then:
git add <resolved-files>
git rebase --continue  # or git commit
```

### Resolution Strategies
1. **Accept theirs**: `git checkout --theirs <file>`
2. **Accept ours**: `git checkout --ours <file>`
3. **Manual merge**: Edit file to combine changes
4. **Discuss**: Talk to the other author if unclear

## Common Git Operations

### Undo Last Commit (not pushed)
```bash
git reset --soft HEAD~1   # Keep changes staged
git reset HEAD~1          # Keep changes unstaged
git reset --hard HEAD~1   # Discard changes
```

### Amend Last Commit
```bash
git commit --amend        # Change message
git commit --amend --no-edit  # Add files to last commit
```

### Interactive Rebase
```bash
git rebase -i HEAD~3      # Edit last 3 commits
# pick, reword, edit, squash, drop
```

### Cherry-pick
```bash
git cherry-pick <commit>  # Apply specific commit
```

### Stash
```bash
git stash                 # Save work in progress
git stash pop             # Restore and remove from stash
git stash apply           # Restore but keep in stash
```

## Git Hygiene

### Before Committing
- Run tests locally
- Run linter/formatter
- Review your own diff
- Check for debug code or console.logs

### Keeping History Clean
- Squash WIP commits before merging
- Rebase feature branches on main
- Use meaningful commit messages
- Avoid merge commits when possible (rebase instead)

## Output Format

When helping with Git:

```
## Current Situation
[What state the repo is in]

## Recommended Action
[Git commands to run]

## Explanation
[Why this approach]

## Alternative
[Other options if applicable]
```
