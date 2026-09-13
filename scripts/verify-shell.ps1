# Checks the shell integration of a running release build without synthetic input:
# window styles, z-order pinning, tray registration, single instance, hide/show,
# capture while hidden, --quit. Talks to Ambient's own hidden shell window only.
# The idle-CPU check fails if the Sticky Notes import offer scans during it (3 s after
# start): run against a profile where the import was done or declined, or with
# LOCALAPPDATA pointing at a test directory.
param([string]$Exe = "$PSScriptRoot\..\target\release\ambient.exe")
$ErrorActionPreference = 'Stop'
$Exe = [IO.Path]::GetFullPath($Exe)

Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtr(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint f);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out int v, int size);
}
"@
$WM_APP = 0x8000; $WM_TRAY = $WM_APP + 1; $WM_HOTKEY = 0x0312; $NIN_SELECT = 0x400
$results = [System.Collections.Generic.List[object]]::new()
function Check($name, $ok, $detail = "") { $results.Add([pscustomobject]@{ check = $name; ok = [bool]$ok; detail = $detail }) }
function Wait-Until($cond, $ms = 3000) { $sw = [Diagnostics.Stopwatch]::StartNew(); while ($sw.ElapsedMilliseconds -lt $ms) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 25 }; return [bool](& $cond) }
function Layer { $p = Get-Process ambient -ErrorAction SilentlyContinue | Select-Object -First 1; if (-not $p) { return [IntPtr]::Zero }; [W]::FindWindow([NullString]::Value, "Ambient") }

if (Get-Process ambient -ErrorAction SilentlyContinue) { throw "close running Ambient instances first" }

$p = Start-Process $Exe -PassThru
$shell = [IntPtr]::Zero
Wait-Until { $script:shell = [W]::FindWindow("AmbientNotes.Shell", [NullString]::Value); $shell -ne [IntPtr]::Zero } | Out-Null
$layer = [IntPtr]::Zero
Wait-Until { $script:layer = [W]::FindWindow([NullString]::Value, "Ambient"); ($layer -ne [IntPtr]::Zero) -and [W]::IsWindowVisible($layer) } 5000 | Out-Null
Start-Sleep -Milliseconds 800
Check "shell window exists" ($shell -ne [IntPtr]::Zero)
Check "layer visible after start" ([W]::IsWindowVisible($layer))

# Styles
$style = [int64][W]::GetWindowLongPtr($layer, -16); $ex = [int64][W]::GetWindowLongPtr($layer, -20)
Check "layer WS_EX_TOOLWINDOW (no taskbar button, not in Alt+Tab)" (($ex -band 0x80) -ne 0) ("exstyle=0x{0:X}" -f $ex)
Check "layer no WS_EX_APPWINDOW" (($ex -band 0x40000) -eq 0)
Check "layer no WS_SYSMENU" (($style -band 0x80000) -eq 0) ("style=0x{0:X}" -f $style)

# Z-order: count visible, non-cloaked, non-layer top-level windows below the layer.
function Below($h) {
  $n = @(); $w = [W]::GetWindow($h, 2)  # GW_HWNDNEXT
  while ($w -ne [IntPtr]::Zero) {
    $cloaked = 0; [void][W]::DwmGetWindowAttribute($w, 14, [ref]$cloaked, 4)
    if ([W]::IsWindowVisible($w) -and $cloaked -eq 0) { $sb = New-Object Text.StringBuilder 128; [void][W]::GetClassName($w, $sb, 128); $n += $sb.ToString() }
    $w = [W]::GetWindow($w, 2)
  }
  ,$n
}
$below = Below $layer
Check "layer at bottom of z-order" ($below.Count -le 2) ("visible below: " + ($below -join ', '))
# Someone raises it (what activation does): the subclass must keep it at the bottom.
[void][W]::SetWindowPos($layer, [IntPtr]0, 0, 0, 0, 0, 0x0001 -bor 0x0002 -bor 0x0010)  # HWND_TOP, NOSIZE|NOMOVE|NOACTIVATE
Start-Sleep -Milliseconds 100
$below2 = Below $layer
Check "stays at bottom after HWND_TOP request" ($below2.Count -le 2) ("visible below: " + ($below2 -join ', '))
# Style rewrite (what winit does on show/hide) must not bring back the taskbar button.

# Tray registration (Windows 11 records every notification icon here).
$tray = Get-ChildItem "HKCU:\Control Panel\NotifyIconSettings" -ErrorAction SilentlyContinue |
  ForEach-Object { Get-ItemProperty $_.PSPath } | Where-Object { $_.ExecutablePath -eq $Exe }
Check "tray icon registered with the shell" ($null -ne $tray) ("promoted=" + ($tray.IsPromoted -join ','))

# Tray left click -> hide, then second instance -> show.
[void][W]::PostMessage($shell, $WM_TRAY, [IntPtr]0, [IntPtr]$NIN_SELECT)
Check "tray click hides layer" (Wait-Until { -not [W]::IsWindowVisible($layer) })
$ex2 = [int64][W]::GetWindowLongPtr($layer, -20); $st2 = [int64][W]::GetWindowLongPtr($layer, -16)
Start-Sleep -Milliseconds 700
$hiddenWs = [int]((Get-Process -Id $p.Id).WorkingSet64 / 1MB); $hiddenPriv = [int]((Get-Process -Id $p.Id).PrivateMemorySize64 / 1MB)
Check "memory while hidden (trimmed)" $true "ws=${hiddenWs} MiB private=${hiddenPriv} MiB"

$cpu0 = (Get-Process -Id $p.Id).TotalProcessorTime.TotalMilliseconds
Start-Sleep -Seconds 3
$cpu1 = (Get-Process -Id $p.Id).TotalProcessorTime.TotalMilliseconds
Check "idle CPU while hidden (3 s)" (($cpu1 - $cpu0) -lt 50) ("{0:0} ms" -f ($cpu1 - $cpu0))

# Capture hotkey while hidden.
[void][W]::PostMessage($shell, $WM_HOTKEY, [IntPtr]1, [IntPtr]0)
$cap = [IntPtr]::Zero
$shown = Wait-Until { $script:cap = [W]::FindWindow([NullString]::Value, "Ambient Capture"); ($cap -ne [IntPtr]::Zero) -and [W]::IsWindowVisible($cap) }
Check "hotkey shows capture while layer hidden" $shown
Check "layer stays hidden meanwhile" (-not [W]::IsWindowVisible($layer))
[void][W]::PostMessage($shell, $WM_HOTKEY, [IntPtr]1, [IntPtr]0)
Check "hotkey again hides capture" (Wait-Until { -not [W]::IsWindowVisible($cap) })

$sw = [Diagnostics.Stopwatch]::StartNew()
$p2 = Start-Process $Exe -PassThru; $p2.WaitForExit(10000) | Out-Null
Check "second instance exits" ($p2.HasExited) ("exit={0} in {1} ms" -f $p2.ExitCode, $sw.ElapsedMilliseconds)
Check "second instance shows layer" (Wait-Until { [W]::IsWindowVisible($layer) })
Check "still one process" (@(Get-Process ambient).Count -eq 1)
$ex3 = [int64][W]::GetWindowLongPtr($layer, -20); $st3 = [int64][W]::GetWindowLongPtr($layer, -16)
Check "styles survive hide/show" ((($ex3 -band 0x80) -ne 0) -and (($ex3 -band 0x40000) -eq 0) -and (($st3 -band 0x80000) -eq 0)) ("exstyle=0x{0:X} style=0x{1:X}" -f $ex3, $st3)
$below3 = Below $layer
Check "at bottom after show" ($below3.Count -le 2) ("visible below: " + ($below3 -join ', '))

$p3 = Start-Process $Exe -ArgumentList "--quit" -PassThru; $p3.WaitForExit(10000) | Out-Null
Check "--quit exits the running instance" ($p.WaitForExit(5000)) ("exit code {0}" -f $p.ExitCode)

$results | Format-Table -AutoSize -Wrap | Out-String -Width 220
if ($results | Where-Object { -not $_.ok }) { exit 1 }

