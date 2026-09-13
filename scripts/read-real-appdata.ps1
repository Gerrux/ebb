# Copies %LOCALAPPDATA%\Ebb\<file> as seen OUTSIDE any MSIX container.
# Processes started from a packaged app (e.g. the Claude desktop app) get
# %LOCALAPPDATA% redirected into Packages\<app>\LocalCache, so the real file that
# an autostarted Ebb writes is invisible to them. A one-off scheduled task
# runs outside the container, copies the file into target\, then is deleted.
param([string]$File = "timing.log", [string]$Dest = "$PSScriptRoot\..\target\real-$File")
$Dest = [IO.Path]::GetFullPath($Dest)
$name = "Ebb read-real-appdata ($env:USERNAME)"
$cmd = "powershell.exe -NoProfile -WindowStyle Hidden -Command Copy-Item -LiteralPath (Join-Path `$env:LOCALAPPDATA 'Ebb\$File') -Destination '$Dest' -Force"
Remove-Item $Dest -ErrorAction SilentlyContinue
schtasks /create /tn $name /sc once /st 00:00 /tr $cmd /f | Out-Null
schtasks /run /tn $name | Out-Null
for ($i = 0; $i -lt 50 -and -not (Test-Path $Dest); $i++) { Start-Sleep -Milliseconds 200 }
schtasks /delete /tn $name /f | Out-Null
if (Test-Path $Dest) { Get-Content $Dest } else { Write-Error "copy did not appear at $Dest" }
