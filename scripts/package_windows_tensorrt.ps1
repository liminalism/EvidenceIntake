[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$LegeOcrPayload,

    [string[]]$ModelPacks = @(),

    [string]$OutputDirectory,
    [UInt64]$VramBudget = 6442450944,
    [string]$Gpu = 'NVIDIA GPU',
    [string]$TensorRTVersion = '10',
    [string]$TensorRTLLMVersion,
    [switch]$SkipBuild,
    [switch]$SkipDoctor
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$workspaceRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutputDirectory) {
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $OutputDirectory = Join-Path $workspaceRoot ".agent\scratch\evidence-trt-package\$stamp"
}

function Resolve-RequiredDirectory {
    param([string]$Path, [string]$Label)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "$Label directory does not exist: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

function Copy-RequiredFile {
    param([string]$Source, [string]$Destination)
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        throw "Required package file does not exist: $Source"
    }
    Copy-Item -LiteralPath $Source -Destination $Destination
}

function Write-JsonNoBom {
    param([object]$Value, [string]$Path, [int]$Depth)
    $json = $Value | ConvertTo-Json -Depth $Depth
    $encoding = [System.Text.UTF8Encoding]::new($false)
    [System.IO.File]::WriteAllText($Path, "$json`r`n", $encoding)
}

$LegeOcrPayload = Resolve-RequiredDirectory $LegeOcrPayload 'Lege OCR payload'
$resolvedPacks = @($ModelPacks | ForEach-Object {
    Resolve-RequiredDirectory $_ 'Evidence TensorRT model pack'
})
$legeTensorRt = Resolve-RequiredDirectory (Join-Path $LegeOcrPayload 'tensorrt') 'Lege TensorRT payload'

$outputParent = Split-Path -Parent $OutputDirectory
if ($outputParent) {
    New-Item -ItemType Directory -Path $outputParent -Force | Out-Null
}
if (Test-Path -LiteralPath $OutputDirectory) {
    if (Get-ChildItem -LiteralPath $OutputDirectory -Force | Select-Object -First 1) {
        throw "Output directory must be empty: $OutputDirectory"
    }
} else {
    New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
}
$OutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path

if (-not $SkipBuild) {
    Push-Location $workspaceRoot
    try {
        cargo build --release -p evidence-trt -p evidence-document -p evidence-audio -p evidence-video
        if ($LASTEXITCODE -eq 0) {
            cargo build --release -p evidence-intake --features gui-winsafe --bin evidence-gui
        }
        if ($LASTEXITCODE -ne 0) {
            throw "Evidence release build failed with exit code $LASTEXITCODE"
        }
    }
    finally {
        Pop-Location
    }
}

$binaryDirectory = Join-Path $OutputDirectory 'bin'
$modelDirectory = Join-Path $OutputDirectory 'models'
$ocrDirectory = Join-Path $OutputDirectory 'lege-ocr'
New-Item -ItemType Directory -Path $binaryDirectory, $modelDirectory, $ocrDirectory | Out-Null

foreach ($binary in @(
    'evidence-trt-broker.exe',
    'evidence-trt-lege-ocr-bridge.exe',
    'evidence-document.exe',
    'evidence-audio.exe',
    'evidence-video.exe',
    'evidence-gui.exe'
)) {
    Copy-RequiredFile (Join-Path $workspaceRoot "target\release\$binary") $binaryDirectory
}
Copy-RequiredFile (Join-Path $PSScriptRoot 'run_windows_tensorrt.ps1') $OutputDirectory
Copy-RequiredFile (Join-Path $PSScriptRoot 'run_windows_document.ps1') $OutputDirectory
Copy-RequiredFile (Join-Path $LegeOcrPayload 'lege-ocr.exe') $ocrDirectory
# The GUI resolves all adapter companions beside itself. Keep the standalone
# Lege payload directory for its support files and place the executable in bin.
Copy-RequiredFile (Join-Path $LegeOcrPayload 'lege-ocr.exe') $binaryDirectory
if (Test-Path -LiteralPath (Join-Path $LegeOcrPayload 'licenses') -PathType Container) {
    Copy-Item -LiteralPath (Join-Path $LegeOcrPayload 'licenses') -Destination $ocrDirectory -Recurse
}

# Turn the already self-contained Lege TensorRT payload into the first broker
# pack. Evidence-document reaches it exclusively through the broker, so the
# package carries only one copy of the native runtime and OCR graphs.
$ocrPackId = 'turbo-ocr'
$ocrPack = Join-Path $modelDirectory $ocrPackId
New-Item -ItemType Directory -Path $ocrPack | Out-Null
Get-ChildItem -LiteralPath $legeTensorRt -Force |
    Copy-Item -Destination $ocrPack -Recurse
$ocrPackBin = Join-Path $ocrPack 'bin'
New-Item -ItemType Directory -Path $ocrPackBin -Force | Out-Null
Copy-RequiredFile `
    (Join-Path $workspaceRoot 'target\release\evidence-trt-ocr-worker.exe') `
    $ocrPackBin

$modelHashes = @(
    'models\det_tiny.onnx',
    'models\rec_tiny.onnx',
    'models\keys_tiny.txt'
) | ForEach-Object {
    $path = Join-Path $ocrPack $_
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Lege TensorRT payload is missing required OCR artifact: $path"
    }
    (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}
$revisionHasher = [System.Security.Cryptography.SHA256]::Create()
try {
    $revisionBytes = [System.Text.Encoding]::UTF8.GetBytes(($modelHashes -join ':'))
    $ocrRevision = ([System.BitConverter]::ToString(
        $revisionHasher.ComputeHash($revisionBytes)
    )).Replace('-', '').ToLowerInvariant()
}
finally {
    $revisionHasher.Dispose()
}

$ocrArtifacts = Get-ChildItem -LiteralPath $ocrPack -File -Recurse | Sort-Object FullName | ForEach-Object {
    $relative = $_.FullName.Substring($ocrPack.Length + 1).Replace('\', '/')
    $role = switch ($relative) {
        'bin/evidence-trt-ocr-worker.exe' { 'worker'; break }
        'bin/turboocr-text.exe' { 'native_worker'; break }
        'models/det_tiny.onnx' { 'detector'; break }
        'models/rec_tiny.onnx' { 'recognizer'; break }
        'models/keys_tiny.txt' { 'dictionary'; break }
        default { 'runtime' }
    }
    [ordered]@{
        role = $role
        path = $relative
        sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
$ocrManifest = [ordered]@{
    schema_version = 1
    id = $ocrPackId
    revision = $ocrRevision
    provider = 'Lege/TurboOCR'
    license = 'MIT'
    runtime = 'tensor_rt'
    worker = 'bin/evidence-trt-ocr-worker.exe'
    worker_args = @(
        'serve',
        '--turbo-worker', 'bin/turboocr-text.exe',
        '--detector', 'models/det_tiny.onnx',
        '--recognizer', 'models/rec_tiny.onnx',
        '--dictionary', 'models/keys_tiny.txt',
        '--runtime-dir', 'runtime'
    )
    operations = @('page_ocr')
    estimated_vram_bytes = [UInt64]2147483648
    artifacts = @($ocrArtifacts)
}
Write-JsonNoBom $ocrManifest (Join-Path $ocrPack 'manifest.json') 6

$seenIds = @{}
$seenIds[$ocrPackId] = $true
foreach ($pack in $resolvedPacks) {
    $manifestPath = Join-Path $pack 'manifest.json'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "Model pack has no manifest.json: $pack"
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if (-not $manifest.id -or $manifest.schema_version -ne 1) {
        throw "Model pack manifest has no schema-1 id: $manifestPath"
    }
    if ($seenIds.ContainsKey($manifest.id)) {
        throw "Duplicate model id in package: $($manifest.id)"
    }
    $seenIds[$manifest.id] = $true
    Copy-Item -LiteralPath $pack -Destination (Join-Path $modelDirectory $manifest.id) -Recurse
}

$runtimeConfig = [ordered]@{
    schema_version = 1
    vram_budget = $VramBudget
    gpu = $Gpu
    tensorrt_version = $TensorRTVersion
    tensorrt_llm_version = $TensorRTLLMVersion
    ocr_model = $ocrPackId
    ocr_revision = $ocrRevision
}
Write-JsonNoBom $runtimeConfig (Join-Path $OutputDirectory 'runtime-config.json') 3

$broker = Join-Path $binaryDirectory 'evidence-trt-broker.exe'
if (-not $SkipDoctor) {
    $doctorArguments = @(
        'doctor', '--models', $modelDirectory,
        '--vram-budget', $VramBudget,
        '--gpu', $Gpu,
        '--tensorrt-version', $TensorRTVersion
    )
    if ($TensorRTLLMVersion) {
        $doctorArguments += @('--tensorrt-llm-version', $TensorRTLLMVersion)
    }
    & $broker @doctorArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Packaged Evidence TensorRT doctor failed with exit code $LASTEXITCODE"
    }

    $doctorEndpoint = "evidence-trt-package-doctor-$PID"
    $serveArguments = @(
        'serve', '--endpoint', $doctorEndpoint,
        '--models', $modelDirectory,
        '--vram-budget', $VramBudget,
        '--gpu', $Gpu,
        '--tensorrt-version', $TensorRTVersion
    )
    if ($TensorRTLLMVersion) {
        $serveArguments += @('--tensorrt-llm-version', $TensorRTLLMVersion)
    }
    $brokerLog = Join-Path $OutputDirectory 'broker-doctor.log'
    $brokerError = Join-Path $OutputDirectory 'broker-doctor-error.log'
    $brokerProcess = Start-Process -FilePath $broker -ArgumentList $serveArguments `
        -PassThru -WindowStyle Hidden -RedirectStandardOutput $brokerLog `
        -RedirectStandardError $brokerError
    try {
        $legeSucceeded = $false
        for ($attempt = 0; $attempt -lt 20 -and -not $legeSucceeded; $attempt++) {
            & (Join-Path $ocrDirectory 'lege-ocr.exe') doctor `
                --backend brokered-tensorrt `
                --broker-bridge (Join-Path $binaryDirectory 'evidence-trt-lege-ocr-bridge.exe') `
                --broker-endpoint $doctorEndpoint `
                --broker-model $ocrPackId `
                --broker-revision $ocrRevision `
                --json
            $legeSucceeded = $LASTEXITCODE -eq 0
            if (-not $legeSucceeded) {
                Start-Sleep -Milliseconds 250
            }
        }
        if (-not $legeSucceeded) {
            throw "Packaged Lege brokered OCR doctor failed"
        }
    }
    finally {
        if (-not $brokerProcess.HasExited) {
            Stop-Process -Id $brokerProcess.Id
        }
        $brokerProcess.WaitForExit()
    }
}

$files = Get-ChildItem -LiteralPath $OutputDirectory -File -Recurse | Sort-Object FullName | ForEach-Object {
    [ordered]@{
        path = $_.FullName.Substring($OutputDirectory.Length + 1).Replace('\', '/')
        bytes = $_.Length
        sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
$packageManifest = [ordered]@{
    schema_version = 1
    product = 'Evidence Intake TensorRT Runtime'
    target = 'windows-x86_64-nvidia'
    model_ids = @($seenIds.Keys | Sort-Object)
    python_runtime = $false
    files = @($files)
}
Write-JsonNoBom $packageManifest (Join-Path $OutputDirectory 'package-manifest.json') 6

Write-Host "Staged Evidence TensorRT package: $OutputDirectory"
Write-Host "Models: $($seenIds.Count); files: $($files.Count)"
