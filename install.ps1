# Naantree Installer for Windows
# Downloads and installs the latest naantree CLI binary.

[CmdletBinding()]
param(
    [string]$Version = $env:NAANTREE_VERSION
)

$ErrorActionPreference = 'Stop'

# Ensure TLS 1.2 is enabled
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo = "itznan/naantree"
$arch = $env:PROCESSOR_ARCHITECTURE

# Architecture check
if ($arch -ne "AMD64" -and $arch -ne "ARM64") {
    Write-Warning "Detected architecture '$arch'. Naantree Windows binaries are built for x86_64/AMD64."
}

# Determine download URL
$assetName = "naantree-windows-x86_64.zip"
if ([string]::IsNullOrWhiteSpace($Version) -or $Version -eq "latest") {
    $downloadUrl = "https://github.com/$repo/releases/latest/download/$assetName"
} else {
    $downloadUrl = "https://github.com/$repo/releases/download/$Version/$assetName"
}

# Determine installation directory:
# Prefers $env:LOCALAPPDATA\Programs\naantree, falls back to $HOME\.naantree\bin
if ($env:LOCALAPPDATA) {
    $installDir = Join-Path $env:LOCALAPPDATA "Programs\naantree"
} else {
    $installDir = Join-Path $HOME ".naantree\bin"
}

if (-not (Test-Path -Path $installDir)) {
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
}

# Create a temporary directory for extraction
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("naantree-install-" + [System.Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
$zipPath = Join-Path $tempDir $assetName
$extractDir = Join-Path $tempDir "extracted"

try {
    Write-Host "Downloading naantree from $downloadUrl..."
    Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -UseBasicParsing

    Write-Host "Extracting archive..."
    Expand-Archive -Path $zipPath -DestinationPath $extractDir -Force

    $exeFile = Get-ChildItem -Path $extractDir -Recurse -Filter "naantree.exe" | Select-Object -First 1
    if (-not $exeFile) {
        throw "naantree.exe was not found inside the downloaded package."
    }

    $targetExe = Join-Path $installDir "naantree.exe"
    Copy-Item -Path $exeFile.FullName -Destination $targetExe -Force
}
finally {
    Remove-Item -Path $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}

# Add to User PATH if not already present
$userPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
$pathEntries = if ($userPath) { $userPath -split ';' } else { @() }
$cleanInstallDir = $installDir.TrimEnd('\', '/')

$alreadyInPath = $false
foreach ($entry in $pathEntries) {
    if ($entry.TrimEnd('\', '/') -ieq $cleanInstallDir) {
        $alreadyInPath = $true
        break
    }
}

if (-not $alreadyInPath) {
    Write-Host "Adding '$installDir' to User PATH..."
    $updatedPath = if ([string]::IsNullOrWhiteSpace($userPath)) {
        $cleanInstallDir
    } else {
        "$userPath;$cleanInstallDir"
    }
    [Environment]::SetEnvironmentVariable("Path", $updatedPath, [EnvironmentVariableTarget]::User)
}

# Update current session PATH so naantree can be invoked immediately
if ($env:Path -notlike "*$cleanInstallDir*") {
    $env:Path = "$cleanInstallDir;$env:Path"
}

# Verify installation
$targetExe = Join-Path $installDir "naantree.exe"
if (-not (Test-Path -Path $targetExe)) {
    throw "Verification failed: $targetExe does not exist."
}

Write-Host ""
Write-Host "==> Installation successful!" -ForegroundColor Green
Write-Host "Binary location: $targetExe"
try {
    $versionOutput = & $targetExe --version
    Write-Host "Version:         $versionOutput"
} catch {
    Write-Warning "Could not execute binary directly: $_"
}

Write-Host ""
Write-Host "Run 'naantree --help' to get started."
