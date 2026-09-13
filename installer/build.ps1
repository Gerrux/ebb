# Builds the release exe, the installer and the portable zip into dist\.
#
#   powershell -File installer\build.ps1
#
# CI runs the same script (.github/workflows/release.yml), so a local build and a
# released one cannot drift apart. Needs Rust (MSVC) and Inno Setup 6.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
  $version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
  Write-Host "== Ebb $version"

  & cargo build --release --locked
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

  $iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
  if (-not $iscc) {
    $iscc = @(
      "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
      "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
      "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
  }
  if (-not $iscc) { throw "Inno Setup 6 not found (winget install JRSoftware.InnoSetup)" }

  New-Item -ItemType Directory -Force dist | Out-Null
  & $iscc /Q "/DAppVersion=$version" installer\ebb.iss
  if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }

  $zip = "dist\ebb-$version-windows-x64.zip"
  Compress-Archive -Force -Path target\release\ebb.exe, LICENSE -DestinationPath $zip

  Get-ChildItem dist | Where-Object { $_.Name -like "*$version*" } |
    ForEach-Object { Write-Host ("done: {0} ({1:N1} MB)" -f $_.FullName, ($_.Length / 1MB)) }
} finally { Pop-Location }
