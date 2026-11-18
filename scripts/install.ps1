[CmdletBinding()]
param(
    [string]$Repo = $(if ($env:TESIE_REPO) { $env:TESIE_REPO } else { "Mushfiqtaief/Tesie" }),
    [string]$InstallDir = $(if ($env:TESIE_INSTALL_DIR) { $env:TESIE_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "Tesie" }),
    [string]$Version = $(if ($env:TESIE_VERSION) { $env:TESIE_VERSION } else { "latest" }),
    [switch]$FromSource,
    [switch]$NoOpen
)

$ErrorActionPreference = "Stop"

function Write-Step([string]$msg) { Write-Host "  • $msg" -ForegroundColor Cyan }
function Write-Done([string]$msg) { Write-Host "  ✓ $msg" -ForegroundColor Green }
function Write-Warn([string]$msg) { Write-Host "  ! $msg" -ForegroundColor Yellow }

$UseSource = $FromSource.IsPresent -or ($env:TESIE_FROM_SOURCE -eq "1")
$SkipOpen = $NoOpen.IsPresent -or ($env:TESIE_NO_OPEN -eq "1")

$TempRoot = Join-Path $env:TEMP ("tesie-install-" + [System.Guid]::NewGuid().ToString("N").Substring(0, 8))
$StageDir = Join-Path $TempRoot "stage"
New-Item -ItemType Directory -Force -Path (Join-Path $StageDir "bin"), (Join-Path $StageDir "www") | Out-Null

function Build-FromSource([string]$SrcRoot) {
    foreach ($cmd in @("go", "cargo")) {
        if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
            throw "Required build tool '$cmd' was not found on PATH."
        }
    }
    $hasBun = [bool](Get-Command bun -ErrorAction SilentlyContinue)
    $hasNpm = [bool](Get-Command npm -ErrorAction SilentlyContinue)
    if (-not $hasBun -and -not $hasNpm) {
        throw "Either 'bun' or 'npm' is required to build the dashboard."
    }

    Write-Step "Building TesieCore (Go network engine)..."
    Push-Location (Join-Path $SrcRoot "mihomo")
    try {
        $env:CGO_ENABLED = "0"
        go build -trimpath -ldflags="-s -w" -o (Join-Path $StageDir "bin\tesie-core.exe") .
        if ($LASTEXITCODE -ne 0) { throw "go build failed" }
    } finally {
        Pop-Location
        Remove-Item Env:\CGO_ENABLED -ErrorAction SilentlyContinue
    }

    Write-Step "Building Tesie dashboard SPA..."
    Push-Location (Join-Path $SrcRoot "frontend")
    try {
        if ($hasBun) {
            bun install
            if ($LASTEXITCODE -ne 0) { throw "bun install failed" }
            bun run build
            if ($LASTEXITCODE -ne 0) { throw "bun run build failed" }
        } else {
            npm install
            if ($LASTEXITCODE -ne 0) { throw "npm install failed" }
            npm run build
            if ($LASTEXITCODE -ne 0) { throw "npm run build failed" }
        }
        Copy-Item -Recurse -Force "dist\*" (Join-Path $StageDir "www")
    } finally {
        Pop-Location
    }

    Write-Step "Building Tesie gateway & daemon (Rust release)..."
    Push-Location (Join-Path $SrcRoot "backend")
    try {
        cargo build --release --bins
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
        Copy-Item -Force "target\release\tesie.exe" (Join-Path $StageDir "bin\tesie.exe")
        Copy-Item -Force "target\release\tesie-daemon.exe" (Join-Path $StageDir "bin\tesie-daemon.exe")
    } finally {
        Pop-Location
    }
}

try {
    Write-Host "Installing Tesie (windows/x64)..." -ForegroundColor White

    $Downloaded = $false
    if (-not $UseSource -and $Repo -ne "OWNER/Tesie") {
        $BaseUrl = if ($Version -eq "latest") {
            "https://github.com/$Repo/releases/latest/download"
        } else {
            "https://github.com/$Repo/releases/download/$Version"
        }
        $ZipName = "tesie-windows-x64.zip"
        $ZipPath = Join-Path $TempRoot $ZipName

        Write-Step "Downloading $BaseUrl/$ZipName..."
        try {
            Invoke-WebRequest -Uri "$BaseUrl/$ZipName" -OutFile $ZipPath -UseBasicParsing
            $SumsPath = Join-Path $TempRoot "SHA256SUMS.txt"
            try {
                Invoke-WebRequest -Uri "$BaseUrl/SHA256SUMS.txt" -OutFile $SumsPath -UseBasicParsing
                $line = Select-String -Path $SumsPath -Pattern ([regex]::Escape($ZipName)) | Select-Object -First 1
                if ($line) {
                    $expected = ($line.Line -split '\s+')[0].ToLowerInvariant()
                    $actual = (Get-FileHash -Path $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
                    if ($actual -ne $expected) {
                        throw "SHA256 checksum mismatch for $ZipName"
                    }
                    Write-Done "Verified SHA256 checksum ($ZipName)"
                }
            } catch {}

            Expand-Archive -Path $ZipPath -DestinationPath $StageDir -Force
            $Downloaded = $true
            Write-Done "Downloaded prebuilt $ZipName"
        } catch {
            Write-Warn "Prebuilt release archive unavailable ($($_.Exception.Message)); falling back to source build."
        }
    }

    if (-not $Downloaded) {
        if ((Test-Path "backend\Cargo.toml") -and (Test-Path "mihomo\go.mod")) {
            Build-FromSource (Get-Location).Path
        } elseif ($Repo -ne "OWNER/Tesie") {
            if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
                throw "git is required to clone https://github.com/$Repo.git"
            }
            $SrcClone = Join-Path $TempRoot "src"
            Write-Step "Cloning https://github.com/$Repo.git..."
            git clone --depth 1 "https://github.com/$Repo.git" $SrcClone
            if ($LASTEXITCODE -ne 0) { throw "git clone failed" }
            Build-FromSource $SrcClone
        } else {
            throw "Set TESIE_REPO=<username>/Tesie or run scripts\install.ps1 from a cloned Tesie repository."
        }
    }

    $ExistingTesie = Join-Path $InstallDir "bin\tesie.exe"
    if (Test-Path $ExistingTesie) {
        Write-Step "Stopping existing Tesie processes cleanly..."
        try { & $ExistingTesie stop | Out-Null } catch {}
        Start-Sleep -Milliseconds 400
    }

    $DestBin = Join-Path $InstallDir "bin"
    $DestWww = Join-Path $InstallDir "www"
    New-Item -ItemType Directory -Force -Path $DestBin, $DestWww | Out-Null

    Copy-Item -Force (Join-Path $StageDir "bin\*") $DestBin
    Remove-Item -Recurse -Force $DestWww -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force -Path $DestWww | Out-Null
    Copy-Item -Recurse -Force (Join-Path $StageDir "www\*") $DestWww

    Get-ChildItem -Path $DestBin -File | ForEach-Object {
        try { Unblock-File -Path $_.FullName -ErrorAction SilentlyContinue } catch {}
    }

    Write-Done "Installed Tesie into $InstallDir"

    Write-Step "Registering global CLI on Windows User PATH..."
    & (Join-Path $DestBin "tesie.exe") install

    Write-Step "Starting Tesie daemon & tray..."
    if ($SkipOpen) {
        & (Join-Path $DestBin "tesie.exe") start
    } else {
        & (Join-Path $DestBin "tesie.exe") start --open
    }

    Write-Host ""
    Write-Done "Tesie is ready! Run 'tesie status' anytime or visit http://127.0.0.1:20129"
} finally {
    if (Test-Path $TempRoot) {
        Remove-Item -Recurse -Force $TempRoot -ErrorAction SilentlyContinue
    }
}
