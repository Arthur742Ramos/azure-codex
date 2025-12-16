Help optimize performance using data-driven analysis.

## Performance Rules

1. **Measure first** - Never optimize without profiling
2. **Fix the bottleneck** - Optimize the slowest part
3. **Verify improvement** - Measure again after changes

## Rust Profiling

```bash
# Build with debug symbols
cargo build --release

# CPU profiling
cargo install flamegraph
cargo flamegraph --bin codex

# Benchmarking
cargo bench
```

## Common Optimizations

### Avoid Unnecessary Allocations
```rust
// Bad: Creates new String
format!("{}", x)

// Good: Use existing &str when possible
x.as_str()
```

### Use Appropriate Collections
- `Vec` for sequences
- `HashMap` for key-value (O(1) lookup)
- `BTreeMap` for sorted keys
- `HashSet` for membership tests

### Avoid Cloning
```rust
// Bad: Unnecessary clone
let x = data.clone();
process(x);

// Good: Borrow or move
process(&data);  // or process(data)
```

### Async Optimization
```rust
// Bad: Sequential
let a = fetch_a().await;
let b = fetch_b().await;

// Good: Concurrent
let (a, b) = tokio::join!(fetch_a(), fetch_b());
```

## Azure Codex Specific

### Hot Paths
- Token acquisition/refresh
- API request/response handling
- TUI rendering loop
- Configuration loading

### Check For
- Repeated Azure CLI calls (cache results)
- Unnecessary API round-trips
- Blocking calls in async context
- Large allocations in render loop

## Output Format

```
## Performance Analysis

### Bottleneck
[What's slow and why]

### Recommended Fix
[Specific optimization]

### Expected Improvement
[Estimated impact]

### Verification
[How to measure improvement]
```

What would you like me to optimize?
