param(
    [string]$SumatraPath
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

if ($SumatraPath) {
    if (-not (Test-Path -LiteralPath $SumatraPath -PathType Leaf)) {
        throw "SumatraPDF executable not found: $SumatraPath"
    }
    $SumatraPath = (Resolve-Path -LiteralPath $SumatraPath).Path
}

Push-Location $root
try {
    $rustInfo = & rustc -vV
    if ($LASTEXITCODE -ne 0) { throw 'Failed to inspect the Rust toolchain.' }
    $hostLine = $rustInfo | Where-Object { $_ -like 'host: *' } | Select-Object -First 1
    if (-not $hostLine) { throw 'Could not determine the Rust host target.' }
    $target = $hostLine.Substring(6).Trim()
    if ($target -notmatch '-windows-') {
        throw 'Run this script on Windows with a Windows Rust toolchain.'
    }

    $metadataJson = & cargo metadata --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Failed to read Cargo metadata.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = $metadata.packages | Where-Object { $_.name -eq 'report-client' } | Select-Object -First 1
    if (-not $package) { throw 'The report-client package was not found.' }

    & cargo build --release --target $target
    if ($LASTEXITCODE -ne 0) { throw 'Windows release build failed.' }

    $exe = Join-Path $metadata.target_directory "$target/release/report-client.exe"
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
        throw "Built executable not found: $exe"
    }

    $bundleDir = Join-Path $metadata.target_directory 'bundle'
    New-Item -ItemType Directory -Force -Path $bundleDir | Out-Null
    $zip = Join-Path $bundleDir "Report-Client-$($package.version)-$target.zip"
    $stage = Join-Path $bundleDir ("windows-stage-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        $files = @()
        $client = Join-Path $stage 'report-client.exe'
        Copy-Item -LiteralPath $exe -Destination $client
        $files += $client
        if ($SumatraPath) {
            $sumatra = Join-Path $stage 'SumatraPDF.exe'
            Copy-Item -LiteralPath $SumatraPath -Destination $sumatra
            $files += $sumatra
        }
        Compress-Archive -Path $files -DestinationPath $zip -Force
    }
    finally {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }

    Write-Host "Built $zip"
}
finally {
    Pop-Location
}
