Help with Azure OpenAI configuration and troubleshooting.

## Azure vs OpenAI

| Aspect | OpenAI | Azure OpenAI |
|--------|--------|--------------|
| Endpoint | api.openai.com | {resource}.openai.azure.com |
| Auth | API Key | Azure Entra ID / API Key |
| Models | Model names | Deployment names |

## Authentication Methods

### 1. Azure CLI (Development)
```bash
az login
az account show
```

Config:
```toml
azure_endpoint = "https://your-resource.openai.azure.com"
model = "your-deployment-name"
```

### 2. Managed Identity (Production)
- System-assigned or User-assigned
- Role: `Cognitive Services OpenAI User`

### 3. Service Principal
```bash
az ad sp create-for-rbac --name "codex-sp"
```

Environment:
```
AZURE_CLIENT_ID=<id>
AZURE_CLIENT_SECRET=<secret>
AZURE_TENANT_ID=<tenant>
```

## Common Issues

### 404 DeploymentNotFound
- Deployment name doesn't match
- Deployment not ready (wait 1-2 min after creation)
- Wrong endpoint URL

### 401 Unauthorized
- Token expired → `az login`
- Wrong tenant
- Missing role assignment

### 429 Rate Limited
- Implement exponential backoff
- Request quota increase

## Debugging

```bash
# Test Azure CLI token
az account get-access-token \
  --resource https://cognitiveservices.azure.com

# Test endpoint
curl https://your-resource.openai.azure.com/openai/deployments \
  -H "Authorization: Bearer $TOKEN"

# List deployments
az cognitiveservices account deployment list \
  --name your-resource \
  --resource-group your-rg
```

## Azure Codex Config

```toml
# ~/.config/codex/config.toml
azure_endpoint = "https://your-resource.openai.azure.com"
model = "gpt-4o"  # This is your deployment name
```

What Azure OpenAI issue do you need help with?
