param(
    [int]$Blocks = 1800,
    [int]$SummaryEveryBlocks = 60,
    [switch]$VerboseBlockLogs
)

$ErrorActionPreference = "Continue"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
$logsDir = Join-Path $repoRoot "logs"
New-Item -ItemType Directory -Force -Path $logsDir | Out-Null

$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$jsonlPath = Join-Path $logsDir "shadow-$timestamp.jsonl"
$stderrPath = Join-Path $logsDir "shadow-$timestamp.stderr.log"

$env:SHADOW_MAX_BLOCKS = "$Blocks"
$env:SHADOW_SUMMARY_EVERY_BLOCKS = "$SummaryEveryBlocks"
$env:SHADOW_VERBOSE_BLOCK_LOGS = if ($VerboseBlockLogs.IsPresent) { "true" } else { "false" }

Write-Host "Starting shadow route run..."
Write-Host "Repo: $repoRoot"
Write-Host "Blocks: $Blocks"
Write-Host "Summary every: $SummaryEveryBlocks"
Write-Host "Verbose block logs: $($env:SHADOW_VERBOSE_BLOCK_LOGS)"
Write-Host "JSONL output: $jsonlPath"
Write-Host "STDERR output: $stderrPath"
Write-Host ""

Push-Location $repoRoot
try {
    $runCmd = "cargo run -p evm_flashloans_l2_arb --bin shadow_route 2> `"$stderrPath`""
    & cmd /c $runCmd | Tee-Object -FilePath $jsonlPath
    $exitCode = $LASTEXITCODE
}
finally {
    Pop-Location
}

Write-Host ""
if ($exitCode -eq 0) {
    Write-Host "Run complete."
    Write-Host "JSONL: $jsonlPath"
    Write-Host "STDERR: $stderrPath"
} else {
    Write-Error "Run failed with exit code $exitCode"
}

exit $exitCode
