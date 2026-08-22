[CmdletBinding()]
param(
    [string]$PackageRoot = $PSScriptRoot,
    [string]$Endpoint = 'evidence-trt'
)

$ErrorActionPreference = 'Stop'
$PackageRoot = (Resolve-Path -LiteralPath $PackageRoot).Path
$configPath = Join-Path $PackageRoot 'runtime-config.json'
if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
    throw "Runtime configuration does not exist: $configPath"
}
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
$arguments = @(
    'serve',
    '--endpoint', $Endpoint,
    '--models', (Join-Path $PackageRoot 'models'),
    '--vram-budget', $config.vram_budget,
    '--gpu', $config.gpu,
    '--tensorrt-version', $config.tensorrt_version
)
if ($config.tensorrt_llm_version) {
    $arguments += @('--tensorrt-llm-version', $config.tensorrt_llm_version)
}
& (Join-Path $PackageRoot 'bin\evidence-trt-broker.exe') @arguments
exit $LASTEXITCODE
