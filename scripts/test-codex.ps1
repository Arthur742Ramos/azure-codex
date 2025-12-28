<#
.SYNOPSIS
    Test runner for Azure Codex - allows running prompts and capturing output.

.DESCRIPTION
    This script provides a testing infrastructure for Azure Codex that allows:
    - Running prompts against the debug build
    - Capturing JSON output for verification
    - Testing different models and configurations
    - Verifying token usage, context bar, and other features

.PARAMETER Prompt
    The prompt to send to Codex.

.PARAMETER Model
    Optional model override (e.g., "gpt-5.2", "claude-3-5-sonnet-20241022").

.PARAMETER Json
    Output events as JSONL for structured parsing.

.PARAMETER OutputFile
    File to write the last agent message to.

.PARAMETER Timeout
    Timeout in seconds (default: 120).

.PARAMETER ConfigDir
    Path to config directory (default: test-config).

.EXAMPLE
    .\test-codex.ps1 -Prompt "Say hello" -Json

.EXAMPLE
    .\test-codex.ps1 -Prompt "What is 2+2?" -Model "claude-3-5-sonnet-20241022" -OutputFile "output.txt"
#>

param(
    [Parameter(Mandatory=$false)]
    [string]$Prompt,

    [Parameter(Mandatory=$false)]
    [string]$Model,

    [Parameter(Mandatory=$false)]
    [switch]$Json,

    [Parameter(Mandatory=$false)]
    [string]$OutputFile,

    [Parameter(Mandatory=$false)]
    [int]$Timeout = 120,

    [Parameter(Mandatory=$false)]
    [string]$ConfigDir = "Q:\src\azure-codex\test-config",

    [Parameter(Mandatory=$false)]
    [switch]$Debug,

    [Parameter(Mandatory=$false)]
    [switch]$Release,

    [Parameter(Mandatory=$false)]
    [switch]$ListModels,

    [Parameter(Mandatory=$false)]
    [switch]$Review,

    [Parameter(Mandatory=$false)]
    [switch]$Uncommitted
)

$ErrorActionPreference = "Stop"

# Paths
$RepoRoot = "Q:\src\azure-codex\azure-codex"
$DebugExe = "$RepoRoot\codex-rs\target\debug\codex.exe"
$ReleaseExe = "$RepoRoot\codex-rs\target\release\codex.exe"

# Select executable
if ($Release) {
    $CodexExe = $ReleaseExe
} else {
    $CodexExe = $DebugExe
}

if (-not (Test-Path $CodexExe)) {
    Write-Error "Codex executable not found at $CodexExe. Build it first with 'cargo build -p codex-cli'"
    exit 1
}

# Set environment for test config
$env:AZURE_CODEX_HOME = $ConfigDir

Write-Host "Using config from: $ConfigDir" -ForegroundColor Cyan
Write-Host "Using executable: $CodexExe" -ForegroundColor Cyan

# Build arguments
$args = @()

if ($Model) {
    $args += "-m"
    $args += $Model
}

if ($Json) {
    $args += "--json"
}

if ($OutputFile) {
    $args += "-o"
    $args += $OutputFile
}

$args += "--skip-git-repo-check"

# Handle different modes
if ($ListModels) {
    # For listing models, we use the TUI which isn't ideal for scripting
    # Instead, let's use a simple prompt that asks about available models
    $Prompt = "List all available models"
}

if ($Review -and $Uncommitted) {
    # Build codex-exec for review
    $ExecExe = "$RepoRoot\codex-rs\target\debug\codex-exec.exe"
    if (-not (Test-Path $ExecExe)) {
        Write-Error "codex-exec not found. Build it with 'cargo build -p codex-exec'"
        exit 1
    }

    $reviewArgs = @("review", "--uncommitted")
    if ($Json) { $reviewArgs += "--json" }
    if ($Model) { $reviewArgs += "-m"; $reviewArgs += $Model }

    Write-Host "Running: $ExecExe $($reviewArgs -join ' ')" -ForegroundColor Yellow
    & $ExecExe @reviewArgs
    exit $LASTEXITCODE
}

if (-not $Prompt) {
    Write-Host "Usage: .\test-codex.ps1 -Prompt 'Your prompt here' [-Model model] [-Json] [-OutputFile file]" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Options:" -ForegroundColor Cyan
    Write-Host "  -Prompt       The prompt to send"
    Write-Host "  -Model        Model override (e.g., 'claude-3-5-sonnet-20241022')"
    Write-Host "  -Json         Output as JSONL"
    Write-Host "  -OutputFile   Write last message to file"
    Write-Host "  -Debug        Use debug build (default)"
    Write-Host "  -Release      Use release build"
    Write-Host "  -Review       Run review mode"
    Write-Host "  -Uncommitted  Review uncommitted changes"
    Write-Host ""
    Write-Host "Examples:" -ForegroundColor Cyan
    Write-Host "  .\test-codex.ps1 -Prompt 'Say hello'"
    Write-Host "  .\test-codex.ps1 -Prompt 'What is 2+2?' -Model 'claude-3-5-sonnet-20241022' -Json"
    Write-Host "  .\test-codex.ps1 -Review -Uncommitted"
    exit 0
}

# Use codex-exec for non-interactive testing
$ExecExe = "$RepoRoot\codex-rs\target\debug\codex-exec.exe"
if ($Release) {
    $ExecExe = "$RepoRoot\codex-rs\target\release\codex-exec.exe"
}

if (-not (Test-Path $ExecExe)) {
    Write-Host "codex-exec not found, building..." -ForegroundColor Yellow
    Push-Location "$RepoRoot\codex-rs"
    if ($Release) {
        cargo build -p codex-exec --release
    } else {
        cargo build -p codex-exec
    }
    Pop-Location
}

Write-Host "Running: $ExecExe $($args -join ' ') '$Prompt'" -ForegroundColor Yellow
Write-Host "---" -ForegroundColor Gray

# Run with timeout
$process = Start-Process -FilePath $ExecExe -ArgumentList ($args + @($Prompt)) -NoNewWindow -PassThru -Wait:$false

$completed = $process.WaitForExit($Timeout * 1000)
if (-not $completed) {
    $process.Kill()
    Write-Error "Process timed out after $Timeout seconds"
    exit 1
}

Write-Host "---" -ForegroundColor Gray
Write-Host "Exit code: $($process.ExitCode)" -ForegroundColor $(if ($process.ExitCode -eq 0) { "Green" } else { "Red" })

if ($OutputFile -and (Test-Path $OutputFile)) {
    Write-Host ""
    Write-Host "Output saved to: $OutputFile" -ForegroundColor Cyan
    Write-Host "Content:" -ForegroundColor Cyan
    Get-Content $OutputFile
}

exit $process.ExitCode
