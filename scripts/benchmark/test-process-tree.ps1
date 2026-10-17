$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'process-tree.ps1')
$origin = [datetime]'2026-01-01T00:00:00Z'
function Entry([int]$id, [int]$parentId, [int]$seconds) {
    [pscustomobject]@{ ProcessId=$id; ParentProcessId=$parentId; CreationDate=$origin.AddSeconds($seconds) }
}
# PID 100 belonged to an exited system parent before the application reused it.
# PID 101 was also reused: its historical child must not enter the application tree.
$snapshot = @(
    (Entry 103 101 9), (Entry 102 101 12), (Entry 8 7 2),
    (Entry 7 100 1), (Entry 100 50 10), (Entry 101 100 11), (Entry 200 99 11)
)
$actual = @((Get-ProcessTreeIds -Snapshot $snapshot -RootId 100) | Sort-Object)
if (($actual -join ',') -ne '100,101,102') { throw "PID reuse regression: $($actual -join ',')" }
$missingRejected = $false
try { $null = Get-ProcessTreeIds -Snapshot $snapshot -RootId 300 }
catch { $missingRejected = $true }
if (-not $missingRejected) { throw 'Missing application root must fail sampling' }
$snapshot += [pscustomobject]@{ ProcessId=104; ParentProcessId=100; CreationDate=$null }
$unknownRejected = $false
try { $null = Get-ProcessTreeIds -Snapshot $snapshot -RootId 100 }
catch { $unknownRejected = $true }
if (-not $unknownRejected) { throw 'Unknown descendant creation time must fail sampling' }
Write-Output 'Process-tree PID reuse, missing-root and timestamp regression checks passed.'
