Help design clean, intuitive APIs.

## REST API Principles

### Resource Naming
```
GET /users              # Collection
GET /users/123          # Single resource
GET /users/123/orders   # Nested resource

# Use nouns, not verbs
GET /users        ✓
GET /getUsers     ✗

# Use plural
/users/123        ✓
/user/123         ✗
```

### HTTP Methods
```
GET     - Read (safe, idempotent)
POST    - Create
PUT     - Replace (idempotent)
PATCH   - Partial update
DELETE  - Remove (idempotent)
```

### Status Codes
```
200 OK           - Success
201 Created      - Resource created
204 No Content   - Success, no body
400 Bad Request  - Invalid input
401 Unauthorized - Auth required
403 Forbidden    - Not allowed
404 Not Found    - Doesn't exist
429 Too Many     - Rate limited
500 Server Error - Unexpected error
```

## Rust API Design

### Builder Pattern
```rust
Client::builder()
    .endpoint(url)
    .timeout(Duration::from_secs(30))
    .build()?
```

### Error Types
```rust
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid response: {0}")]
    Parse(String),
}
```

### Return Types
```rust
// Good: Explicit Result
pub fn fetch() -> Result<Data, ApiError>

// Good: Option for optional data
pub fn find() -> Option<Data>
```

## Azure Codex Specific

### API Patterns Used
- Azure OpenAI Chat Completions API
- Azure Resource Management API
- Azure CLI for auth token acquisition

### Design Considerations
- Support both Azure and OpenAI endpoints
- Handle Azure-specific auth (Entra ID)
- Rate limiting and retry logic
- Streaming responses

What API would you like to design or review?
