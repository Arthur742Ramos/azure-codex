Review the code changes using this systematic approach:

## Review Checklist

### 1. Correctness & Logic
- Check for off-by-one errors, null/None issues, edge cases
- Verify error handling is comprehensive
- Look for race conditions in async code

### 2. Security
- Check for injection vulnerabilities
- Verify input validation
- Look for hardcoded secrets

### 3. Performance
- Identify unnecessary allocations
- Check for N+1 patterns
- Review algorithm efficiency

### 4. Code Quality (Azure Codex specific)
- Imports are individual (not grouped)
- No `.unwrap()` or `.expect()` in production code
- Format strings use inlined variables: `format!("{x}")`
- No `.yellow()` or `Color::Rgb()` in TUI code
- Use `tracing` for logging, not `println!`

## Output Format

```
## Summary
[One paragraph overview]

## Critical Issues
[Must-fix items with file:line references]

## Suggestions
[Improvements]

## Positive Observations
[What's done well]
```

Now review the current changes using `/diff` or the files I specify.
