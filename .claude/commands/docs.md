Help me write clear documentation.

## Documentation Types

### API Documentation (Rust doc comments)
```rust
/// Brief one-line description.
///
/// Longer description explaining behavior,
/// edge cases, and important details.
///
/// # Arguments
/// * `param` - Description
///
/// # Returns
/// Description of return value
///
/// # Errors
/// When and why this function returns an error
///
/// # Examples
/// ```
/// let result = my_function(arg);
/// ```
pub fn my_function(param: Type) -> Result<Output, Error>
```

### README Structure
```markdown
# Project Name
Brief description.

## Quick Start
Minimal steps to get running.

## Installation
Detailed setup instructions.

## Usage
Common examples with code.

## Configuration
Available options.

## Contributing
How to contribute.
```

### Code Comments (when to use)
- Complex algorithms (explain "why")
- Non-obvious business rules
- Workarounds with context
- TODO/FIXME with issue references

### When NOT to Comment
- Obvious code
- What the code does (should be clear from code)
- Outdated information

## Azure Codex Specific

Document:
- Azure authentication methods
- Configuration options in config.toml
- Differences from upstream OpenAI Codex
- Windows-specific considerations

What would you like me to document?
