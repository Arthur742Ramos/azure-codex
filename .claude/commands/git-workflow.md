Help with Git operations and best practices.

## Commit Messages

### Format
```
<type>(<scope>): <subject>

<body>

<footer>
```

### Types
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation
- `refactor`: Code restructuring
- `test`: Adding tests
- `chore`: Maintenance

### Example
```
feat(auth): add Azure Managed Identity support

Implement support for Azure Managed Identity authentication
as an alternative to Azure CLI tokens.

Closes #123
```

## Branch Strategy

```
main              # Production-ready
feature/add-xyz   # New features
fix/bug-name      # Bug fixes
```

## Common Operations

### Undo Last Commit (not pushed)
```bash
git reset --soft HEAD~1   # Keep changes staged
git reset HEAD~1          # Keep changes unstaged
```

### Amend Last Commit
```bash
git commit --amend
git commit --amend --no-edit  # Just add files
```

### Rebase on Main
```bash
git fetch origin
git rebase origin/main
```

### Resolve Conflicts
```bash
# After conflict markers appear
# Edit files to resolve
git add <resolved-files>
git rebase --continue
```

### Cherry-pick
```bash
git cherry-pick <commit-hash>
```

## PR Best Practices

### Title
```
feat(auth): add OAuth2 support
```

### Description
```markdown
## Summary
Brief description of changes.

## Changes
- Change 1
- Change 2

## Testing
How this was tested.

## Checklist
- [ ] Tests added
- [ ] Docs updated
```

## Azure Codex Workflow

```bash
# Create feature branch
git checkout -b feature/my-feature

# Make changes, commit
git add .
git commit -m "feat(scope): description"

# Push and create PR
git push -u origin feature/my-feature
```

What Git operation do you need help with?
