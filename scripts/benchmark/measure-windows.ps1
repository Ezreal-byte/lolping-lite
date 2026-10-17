param(
    [int]$RootProcess,
    [ValidateRange(1000,60000)][int]$DurationMs = 6000,
    [switch]$Hardware
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

if ($Hardware) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class LolPingBenchmarkDisplays {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct INFO { public int Size; public RECT Monitor, Work; public uint Flags; }
    public delegate bool Callback(IntPtr monitor, IntPtr dc, ref RECT rect, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumDisplayMonitors(IntPtr dc, IntPtr clip, Callback callback, IntPtr data);
    [DllImport("user32.dll")] static extern bool GetMonitorInfo(IntPtr monitor, ref INFO info);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("shcore.dll")] static extern int GetScaleFactorForMonitor(IntPtr monitor, out int scale);
    public static object[] Read() {
        var previous = SetThreadDpiAwarenessContext(new IntPtr(-4));
        var displays = new List<object>();
        try {
            EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, (IntPtr monitor, IntPtr dc, ref RECT rect, IntPtr data) => {
                var info = new INFO { Size = Marshal.SizeOf<INFO>() };
                if (GetMonitorInfo(monitor, ref info)) {
                    int scale; if (GetScaleFactorForMonitor(monitor, out scale) != 0) scale = 0;
                    displays.Add(new { width = info.Monitor.Right-info.Monitor.Left, height = info.Monitor.Bottom-info.Monitor.Top,
                        scalePercent = scale, primary = (info.Flags & 1) != 0 });
                }
                return true;
            }, IntPtr.Zero);
        } finally { SetThreadDpiAwarenessContext(previous); }
        return displays.ToArray();
    }
}
'@
    $os = Get-CimInstance Win32_OperatingSystem
    $cpu = @(Get-CimInstance Win32_Processor)
    [pscustomobject]@{
        os = $os.Caption
        osVersion = $os.Version
        cpu = ($cpu | Select-Object -First 1).Name.Trim()
        logicalProcessors = ($cpu | Measure-Object NumberOfLogicalProcessors -Sum).Sum
        memoryMiB = [Math]::Round([double]$os.TotalVisibleMemorySize / 1024, 0)
        displays = @([LolPingBenchmarkDisplays]::Read())
    } | ConvertTo-Json -Depth 5 -Compress
    exit 0
}

function Read-Tree {
    $all = @(Get-CimInstance Win32_Process)
    $ids = [Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($RootProcess)
    do {
        $changed = $false
        foreach ($entry in $all) {
            if ($ids.Contains([int]$entry.ParentProcessId) -and $ids.Add([int]$entry.ProcessId)) { $changed = $true }
        }
    } while ($changed)
    $rows = @(foreach ($entry in $all) {
        if ($ids.Contains([int]$entry.ProcessId)) {
            $sample = Get-Process -Id $entry.ProcessId -ErrorAction SilentlyContinue
            if ($sample) {
                [pscustomobject]@{
                    identity = "$($entry.ProcessId):$($entry.CreationDate.ToUniversalTime().Ticks)"
                    name = $sample.ProcessName
                    workingSetBytes = $sample.WorkingSet64
                    privateCommitBytes = $sample.PrivateMemorySize64
                    cpuSeconds = $sample.TotalProcessorTime.TotalSeconds
                }
            }
        }
    })
    if ($rows.Count -eq 0) { throw 'Application process tree is empty' }
    return $rows
}
$first = @(Read-Tree)
$timer = [Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Milliseconds $DurationMs
$last = @(Read-Tree)
$timer.Stop()
$before = @{}
foreach ($process in $first) { $before[$process.identity] = $process.cpuSeconds }
$cpuDelta = 0.0
foreach ($process in $last) {
    if ($before.ContainsKey($process.identity)) { $cpuDelta += [Math]::Max(0, $process.cpuSeconds - $before[$process.identity]) }
    else { $cpuDelta += $process.cpuSeconds }
}
# Identities, executable paths and command lines never enter the public report.
$publicRows = @($last | Select-Object name, workingSetBytes, privateCommitBytes)
[pscustomobject]@{
    sampledAt = [DateTimeOffset]::UtcNow.ToString('o')
    elapsedSeconds = $timer.Elapsed.TotalSeconds
    workingSetBytes = ($last | Measure-Object workingSetBytes -Sum).Sum
    privateCommitBytes = ($last | Measure-Object privateCommitBytes -Sum).Sum
    cpuOneCorePercent = 100.0 * $cpuDelta / $timer.Elapsed.TotalSeconds
    processCount = $last.Count
    exitedDuringWindow = @($first | Where-Object { $identity=$_.identity; -not ($last | Where-Object identity -eq $identity) }).Count
    processes = $publicRows
} | ConvertTo-Json -Depth 5 -Compress
