# Gets an unattended run to the world: while the title is still in its menus
# (the frame-rate line in -Log draws fewer than 100 times a frame), the title's
# window is brought to the front every 5 seconds and Enter (the keyboard's A)
# pressed, as the account picker asks; it stops at the first world frame or
# after -Seconds. If Windows does not hand the title the focus, the Enter goes
# to the window in front: keep the PC to itself while it runs.
#   scripts\press-start.ps1 -Log <the run's log> [-Seconds 180]
param([Parameter(Mandatory)][string]$Log, [int]$Seconds = 180)

Add-Type -Namespace Victory -Name Keys -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
[DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
'@
$started = Get-Date
while (((Get-Date) - $started).TotalSeconds -lt $Seconds) {
    Start-Sleep -Seconds 5
    $last = Select-String -LiteralPath $Log -Pattern 'victory_gpu::host: [0-9.]+ fps, .*?([0-9]+) draws/frame' -ErrorAction SilentlyContinue |
        Select-Object -Last 1
    if ($last -and [int]$last.Matches[0].Groups[1].Value -ge 1000) { break }
    $title = Get-Process victory -ErrorAction SilentlyContinue | Where-Object MainWindowHandle -ne 0 | Select-Object -First 1
    if (-not $title) { continue }
    [void][Victory.Keys]::SetForegroundWindow($title.MainWindowHandle)
    Start-Sleep -Milliseconds 200
    [Victory.Keys]::keybd_event(0x0D, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 120
    [Victory.Keys]::keybd_event(0x0D, 0, 2, [UIntPtr]::Zero)
}
