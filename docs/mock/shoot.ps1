# Renders docs/screenshots/*.png from docs/mock/index.html with headless Chrome
# (Edge as a fallback). Run from anywhere:
#   powershell -File docs\mock\shoot.ps1            # every scene
#   powershell -File docs\mock\shoot.ps1 layer      # some scenes, both languages
# Russian: <name>.png, English: <name>-en.png; social.png is English only.

param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Only)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$page = 'file:///' + ((Join-Path $repo 'docs\mock\index.html') -replace '\\', '/')
$out = Join-Path $repo 'docs\screenshots'
New-Item -ItemType Directory -Force $out | Out-Null

$browser = @(
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $browser) { throw 'Chrome or Edge not found' }

# name, width, height, scale, languages
$scenes = @(
    @('layer',   1280, 592, 2,   @('ru', 'en')),
    @('cards',   780,  400, 2,   @('ru', 'en')),
    @('capture', 720,  212, 2,   @('ru', 'en')),
    @('search',  760,  556, 2,   @('ru', 'en')),
    @('styles',  1280, 764, 2,   @('ru', 'en')),
    @('styles-light', 1280, 764, 2, @('ru', 'en')),
    @('import',  1352, 498, 2,   @('ru', 'en')),
    @('social',  1280, 640, 1,   @('en'))
)

# Chrome logs harmless errors to stderr; PowerShell 5.1 would stop on them.
$ErrorActionPreference = 'Continue'
# Native programs choke on the long session paths: run from a short directory.
Push-Location $env:TEMP
try {
    foreach ($s in $scenes) {
        $name, $w, $h, $scale, $langs = $s
        if ($Only -and $Only -notcontains $name) { continue }
        foreach ($lang in $langs) {
            $file = if ($name -eq 'social') { 'social.png' } elseif ($lang -eq 'ru') { "$name.png" } else { "$name-$lang.png" }
            $target = Join-Path $out $file
            & $browser --headless=new --hide-scrollbars --disable-gpu --force-device-scale-factor=$scale `
                "--window-size=$w,$h" "--screenshot=$target" --virtual-time-budget=1500 `
                "${page}?scene=$name&lang=$lang" 2>$null | Out-Null
            if (-not (Test-Path $target)) { throw "$file was not written" }
            Write-Host "$file  ($w x $h @ $scale)"
        }
    }
} finally {
    Pop-Location
}
