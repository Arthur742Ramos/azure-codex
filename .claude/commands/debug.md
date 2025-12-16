Help me debug this issue using systematic investigation.

## Debugging Process

1. **Reproduce** - Get exact steps and error message
2. **Hypothesize** - Form theories about the cause
3. **Investigate** - Test each hypothesis
4. **Fix** - Make minimal change to resolve
5. **Verify** - Confirm fix and add regression test

## Rust Debugging Tools

```bash
# Verbose output
RUST_BACKTRACE=1 cargo run
RUST_LOG=debug cargo run
RUST_LOG=codex_core=debug cargo run

# Fast checks
cargo check
cargo clippy
```

## Common Rust Errors

### Borrow Checker
```
cannot borrow `x` as mutable because it is also borrowed as immutable
```
→ Restructure borrows, clone, or use RefCell

### Option/Result
```
called `Option::unwrap()` on a `None` value
```
→ Use `?`, `ok_or()`, `unwrap_or_default()`

### Lifetimes
```
`x` does not live long enough
```
→ Extend lifetime, clone, or use 'static

## Azure Codex Specific

### Auth Issues
```bash
az account show
az account get-access-token --resource https://cognitiveservices.azure.com
```

### API Issues
- 404: Deployment name mismatch
- 401: Token expired, re-run `az login`
- 429: Rate limited, implement backoff

## What I Need From You

1. Full error message and backtrace
2. Steps to reproduce
3. What you expected vs what happened
4. Recent changes that might be related

Share the error and I'll help investigate.
