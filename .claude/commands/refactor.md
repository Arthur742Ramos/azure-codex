Help me refactor code safely using incremental changes.

## Golden Rules
1. One change at a time
2. Verify after each change: `cargo check && cargo test`
3. Never change behavior while refactoring
4. Keep tests green throughout

## Common Refactorings

### Extract Function
Move repeated/complex code into a focused function

### Extract Module
Split large files into focused modules:
```rust
// src/auth/mod.rs
mod azure;
mod config;
pub use azure::AzureAuth;
```

### Replace Conditional with Match
```rust
// Before: if/else chain
// After: match expression
```

### Early Returns
Reduce nesting with guard clauses:
```rust
let value = input.ok_or(Error::Missing)?;
if !value.is_valid() {
    return Err(Error::Invalid);
}
// main logic here
```

## Azure Codex Code Smells to Fix
- Functions > 50 lines
- Nested if statements (use `if let && let`)
- String errors instead of typed errors
- Manual `map` patterns instead of `.map()`
- Redundant clones

## Process

1. **Analyze**: Read the code and identify the smell
2. **Plan**: Break into small, safe steps
3. **Execute**: One change, then verify
4. **Repeat**: Continue until done

For each step, I'll tell you what to change, then you run:
```bash
cargo check && cargo test
```

What code would you like to refactor?
