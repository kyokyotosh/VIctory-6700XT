# The README's milestones for one or more run logs (the diagnostics build's output).
#   scripts\fps-milestones.ps1 <log> [<log> ...]
# A sample is a frame-rate line ("N fps, M ms/frame ..., D draws/frame"), written
# every 5 seconds; it is a world sample when it draws 1,000 times a frame or more
# (menus, videos and loading screens draw a few dozen). Per log: the world samples,
# their seconds, whether the device was lost, and for each target the samples at or
# above it. Range N: one world sample reaches N. Steady N: 90% of the world samples
# are N or more over 90 s or more. A milestone needs two logs of the same build and
# settings, with no device loss, that both meet it (README.md, "Milestones").
param([Parameter(Mandatory, ValueFromRemainingArguments)][string[]]$Logs)

$targets = 5, 10, 15, 20, 25, 30
foreach ($log in $Logs) {
    $text = Get-Content -LiteralPath $log
    $lost = [bool]($text | Select-String -SimpleMatch 'device removed' -Quiet)
    $samples = foreach ($line in $text) {
        if ($line -match 'victory_gpu::host: ([0-9.]+) fps, ([0-9.]+) ms/frame .*?, ([0-9]+) draws/frame') {
            if ([int]$Matches[3] -ge 1000) { [double]$Matches[1] }
        }
    }
    $samples = @($samples)
    $seconds = 5 * $samples.Count
    Write-Host ("{0}: {1} world samples (~{2} s), device {3}" -f $log, $samples.Count, $seconds, $(if ($lost) { 'LOST' } else { 'kept' }))
    if ($samples.Count -eq 0) { continue }
    Write-Host ('  samples: ' + (($samples | ForEach-Object { $_.ToString('0.0') }) -join ' '))
    foreach ($n in $targets) {
        $at = @($samples | Where-Object { $_ -ge $n }).Count
        $share = $at / $samples.Count
        $range = (-not $lost) -and $at -gt 0
        $steady = (-not $lost) -and $seconds -ge 90 -and $share -ge 0.9
        Write-Host ("  {0,2} fps: {1,3} of {2} at or above ({3:P0}); range {4}, steady {5}" -f $n, $at, $samples.Count, $share,
            $(if ($range) { 'met' } else { 'no' }), $(if ($steady) { 'met' } else { 'no' }))
    }
}
