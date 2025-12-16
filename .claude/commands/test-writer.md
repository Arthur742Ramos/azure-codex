Write comprehensive tests following these Rust testing patterns:

## Test Structure

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #![allow(clippy::unwrap_used)]

    #[test]
    fn test_[unit]_[scenario]_[expected]() {
        // Arrange

        // Act

        // Assert
    }
}
```

## Async Tests
```rust
#[tokio::test]
async fn test_async_function() {
    let result = async_function().await;
    assert!(result.is_ok());
}
```

## Edge Cases to Cover
- Empty inputs (empty string, empty vec, None)
- Boundary values (0, -1, MAX, MIN)
- Invalid inputs (wrong type, malformed data)
- Error paths (network failures, timeouts)
- Unicode and special characters

## Azure Codex Specific
- Authentication edge cases (expired tokens, missing credentials)
- API errors (404, 401, 429)
- Configuration edge cases (missing fields, invalid TOML)
- TUI edge cases (small terminal, long text)

## Commands
```bash
cargo test                      # Run all tests
cargo test test_name            # Run specific test
cargo test -- --nocapture       # See output
```

Now write tests for the code I specify. Include the `#![allow(clippy::unwrap_used)]` attribute for test modules.
