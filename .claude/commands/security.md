Perform a security review using OWASP guidelines.

## Security Checklist

### 1. Injection
- [ ] No string concatenation in queries/commands
- [ ] User input sanitized before use
- [ ] No dynamic code evaluation (eval)

### 2. Authentication
- [ ] Tokens stored securely
- [ ] Credentials not logged
- [ ] Session management is secure

### 3. Sensitive Data
- [ ] No hardcoded secrets
- [ ] Sensitive data encrypted
- [ ] Logs don't contain secrets
- [ ] `.env` files in `.gitignore`

### 4. Access Control
- [ ] Authorization checks on all endpoints
- [ ] Users can only access their data
- [ ] Path traversal prevented

### 5. Security Misconfiguration
- [ ] No default credentials
- [ ] Debug mode disabled in prod
- [ ] Error messages don't leak info

### 6. Dependencies
- [ ] No known vulnerabilities
- [ ] Dependencies up to date
```bash
cargo audit
cargo outdated
```

## Azure Codex Specific

### Check For
- Azure credentials in source code
- API keys committed to git
- Tokens in logs or error messages
- Insecure Azure CLI usage

### Secure Patterns
```rust
// Good: Environment variable
let key = std::env::var("AZURE_API_KEY")?;

// Bad: Hardcoded
let key = "sk-abc123...";
```

## Output Format

```
## Security Assessment

### Critical (Fix Immediately)
[Issues that could be exploited]

### High Risk
[Significant concerns]

### Medium Risk
[Should be addressed]

### Recommendations
[Best practice improvements]
```

What code should I review for security issues?
