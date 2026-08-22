[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Pdf,

    [Parameter(Mandatory = $true)]
    [string]$Case,

    [Parameter(Mandatory = $true)]
    [string]$Production,

    [Parameter(Mandatory = $true)]
    [string]$Source,

    [Parameter(Mandatory = $true)]
    [string]$ArtifactsDirectory,

    [string]$PackageRoot = $PSScriptRoot,
    [string]$Output = '-',
    [string]$LogicalName,
    [ValidateSet('contemporaneous', 'after-event', 'mixed', 'unknown')]
    [string]$TemporalRelation = 'unknown'
)

$ErrorActionPreference = 'Stop'
$PackageRoot = (Resolve-Path -LiteralPath $PackageRoot).Path
$config = Get-Content -LiteralPath (Join-Path $PackageRoot 'runtime-config.json') -Raw |
    ConvertFrom-Json
$arguments = @(
    'process', $Pdf,
    '--case', $Case,
    '--production', $Production,
    '--source', $Source,
    '--artifacts-dir', $ArtifactsDirectory,
    '--lege-ocr', (Join-Path $PackageRoot 'lege-ocr\lege-ocr.exe'),
    '--broker-bridge', (Join-Path $PackageRoot 'bin\evidence-trt-lege-ocr-bridge.exe'),
    '--broker-endpoint', 'evidence-trt',
    '--broker-model', $config.ocr_model,
    '--broker-revision', $config.ocr_revision,
    '--temporal-relation', $TemporalRelation,
    '--output', $Output
)
if ($LogicalName) {
    $arguments += @('--logical-name', $LogicalName)
}
& (Join-Path $PackageRoot 'bin\evidence-document.exe') @arguments
exit $LASTEXITCODE
