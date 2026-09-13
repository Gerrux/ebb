# Release benchmark: launches each variant with EBB_BENCH set, one warm-up then interleaved runs.
# Example:
#   .\scripts\bench.ps1 -Variants 'glow|target\release\ebb.exe|','wgpu|target\wgpu\release\ebb.exe|'
# Variant names must not contain commas.
# Variants: "name|exe|ENV=val;ENV=val". Interleaved runs, warm-up discarded, median [min-max].
param([string[]]$Variants, [int]$Runs = 5, [string]$Out = "$env:TEMP\ebb-bench.csv")
Remove-Item $Out -ErrorAction SilentlyContinue
$parsed = foreach ($v in $Variants) {
    $name, $exe, $envs = $v -split '\|', 3
    [pscustomobject]@{ Name = $name; Exe = $exe; Env = @(if ($envs) { $envs -split ';' | Where-Object { $_ } }) }
}
function Run($v, $csv) {
    foreach ($e in $v.Env) { $k, $val = $e -split '=', 2; Set-Item "Env:$k" $val }
    $env:EBB_BENCH = $csv
    $p = Start-Process -FilePath $v.Exe -PassThru
    if (-not $p.WaitForExit(30000)) { Stop-Process -Id $p.Id -Force; Write-Host "timeout $($v.Name)" }
    Remove-Item Env:EBB_BENCH
    foreach ($e in $v.Env) { $k = ($e -split '=', 2)[0]; Remove-Item "Env:$k" }
    # Tag the row just written with the variant name.
    $lines = Get-Content $csv
    $lines[-1] = $v.Name + ($lines[-1].Substring($lines[-1].IndexOf(',')))
    Set-Content $csv $lines
}
$warm = "$env:TEMP\ebb-bench-warmup.csv"
foreach ($v in $parsed) { Run $v $warm }
for ($i = 1; $i -le $Runs; $i++) { foreach ($v in $parsed) { Run $v $Out; Start-Sleep -Milliseconds 400 } }
# The header comes from the first row ever written: list the newest build first when
# comparing against older builds that write fewer columns.
$cols = 'proc_ms','ws_first','priv_first','ws_idle','priv_idle','cpu_idle_ms','latency_ms','ws_capture','priv_capture','search_latency_ms','ws_search','priv_search'
Import-Csv $Out | Group-Object label | ForEach-Object {
    $o = [ordered]@{ variant = $_.Name; n = $_.Count }
    foreach ($col in $cols) {
        $vals = @($_.Group | ForEach-Object {
            $v = 0.0
            if ([double]::TryParse([string]$_.$col, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref]$v) -and -not [double]::IsNaN($v)) { $v }
        } | Sort-Object)
        $o[$col] = if ($vals.Count) { "{0:0.#}" -f $vals[[int][math]::Floor($vals.Count / 2)] } else { "-" }
    }
    [pscustomobject]$o
} | Format-Table -AutoSize | Out-String -Width 300


