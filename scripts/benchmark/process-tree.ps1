# ParentProcessId remains set after a parent exits and Windows can reuse its PID.
# A live child must have been created no earlier than its current live parent.
function Get-ProcessTreeIds {
    param([object[]]$Snapshot, [int]$RootId)
    $byId = @{}
    foreach ($entry in $Snapshot) { $byId[[int]$entry.ProcessId] = $entry }
    if (-not $byId.ContainsKey($RootId) -or -not $byId[$RootId].CreationDate) {
        throw 'Application root process is missing or has no creation timestamp'
    }
    $ids = [Collections.Generic.HashSet[int]]::new()
    [void]$ids.Add($RootId)
    do {
        $changed = $false
        foreach ($entry in $Snapshot) {
            $id = [int]$entry.ProcessId
            $parentId = [int]$entry.ParentProcessId
            if ($ids.Contains($id) -or -not $ids.Contains($parentId)) { continue }
            if (-not $entry.CreationDate) { throw 'Potential descendant has no creation timestamp' }
            if ($entry.CreationDate -lt $byId[$parentId].CreationDate) { continue }
            if ($ids.Add($id)) { $changed = $true }
        }
    } while ($changed)
    return ,$ids
}
