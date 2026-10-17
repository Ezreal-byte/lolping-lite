param([switch]$Execute)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This installer acceptance test requires PowerShell 7 on Windows.' }
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$version = (Get-Content -LiteralPath (Join-Path $repo 'package.json') -Raw -Encoding utf8 | ConvertFrom-Json).version
$installer = Join-Path $repo "dist/LOLPing-Lite-$version-windows-x64-Setup.exe"
$productKey = 'HKCU:\Software\LOLPing Lite'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\LOLPing Lite'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$shortcutDir = Join-Path ([Environment]::GetFolderPath('Programs')) 'LOLPing Lite'
$defaultInstall = Join-Path $env:LOCALAPPDATA 'Programs/LOLPing Lite'
$userData = Join-Path $env:LOCALAPPDATA 'EzrealByte/LOLPingLite/data'
$reportPath = Join-Path $repo 'dist/windows-installer-report.json'

function Has-RunValue {
    return $null -ne (Get-ItemProperty -LiteralPath $runKey -Name 'LOLPing Lite' -ErrorAction SilentlyContinue)
}
function Write-Report($value) {
    $value | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding utf8NoBOM
}
function Check($condition, [string]$message) {
    if (-not $condition) { throw $message }
}
function Wait-Until([scriptblock]$test, [string]$message) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        if (& $test) { return }
        Start-Sleep -Milliseconds 150
    } while ([DateTime]::UtcNow -lt $deadline)
    throw $message
}
function Run-Setup([string]$file, [string[]]$arguments) {
    $process = Start-Process -FilePath $file -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait
    Check ($process.ExitCode -eq 0) "Installer process exited with code $($process.ExitCode)"
}

# Never replace or uninstall an existing user installation, including portable
# use of the normal data directory. This test owns only its fresh registry keys.
$existing = @()
foreach ($path in @($productKey, $uninstallKey, $shortcutDir, $defaultInstall, $userData)) {
    if (Test-Path -LiteralPath $path) { $existing += 'An existing application installation or user-data path was detected.'; break }
}
if (Has-RunValue) { $existing += 'An existing LOLPing Lite startup entry was detected.' }
if ($existing.Count) {
    $report = @{ version = $version; target = 'windows-x64'; skipped = $true; reason = $existing; changedExistingInstallation = $false }
    Write-Report $report
    $report | ConvertTo-Json
    exit 0
}
Check (Test-Path -LiteralPath $installer) 'Build the Windows installer before testing it.'
if (-not $Execute) {
    Write-Output 'Ready: no existing installation found. Use -Execute after other application/benchmark tests finish.'
    exit 0
}

$testId = 'installer-' + [guid]::NewGuid().ToString('N')
$testBase = [IO.Path]::GetFullPath((Join-Path $repo '.test-data'))
$sandbox = [IO.Path]::GetFullPath((Join-Path $testBase $testId))
Check ($sandbox.StartsWith($testBase + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) 'Installer test path escaped .test-data.'
if (Test-Path -LiteralPath $testBase) { Check (-not ((Get-Item -LiteralPath $testBase).Attributes -band [IO.FileAttributes]::ReparsePoint)) '.test-data must not be a symlink or junction.' }
$installDir = Join-Path $sandbox 'app'
# NSIS /D is a special unquoted, final command-line argument. Keep this test's
# generated location free of whitespace instead of relying on shell quoting.
Check ($installDir -notmatch '\s') 'Installer test repository path must not contain spaces.'
$marker = Join-Path $userData 'installer-acceptance-marker.txt'
$markerText = 'LOLPing Lite installer acceptance test ' + $testId
$createdDataDirs = @($userData, (Split-Path $userData), (Split-Path (Split-Path $userData))) | Where-Object { -not (Test-Path -LiteralPath $_) }
$checks = [Collections.Generic.List[string]]::new()
$installed = $false
$failure = $null
try {
    New-Item -ItemType Directory -Path $sandbox -Force | Out-Null
    New-Item -ItemType Directory -Path $userData -Force | Out-Null
    Set-Content -LiteralPath $marker -Value $markerText -Encoding utf8NoBOM -NoNewline

    Run-Setup $installer @('/S', "/D=$installDir")
    $installed = $true
    $exe = Join-Path $installDir 'lolping-lite.exe'
    Check (Test-Path -LiteralPath $exe) 'Installed application executable is missing.'
    $fileInfo = (Get-Item -LiteralPath $exe).VersionInfo
    Check ($fileInfo.ProductName -eq 'LOLPing Lite') 'Executable product name is incorrect.'
    Check ($fileInfo.ProductVersion -eq $version) 'Executable product version is incorrect.'
    Check ((Get-ItemProperty -LiteralPath $productKey).InstallDir -eq $installDir) 'Install directory registry entry is incorrect.'
    Check ((Get-ItemProperty -LiteralPath $uninstallKey).DisplayVersion -eq $version) 'Uninstaller version registry entry is incorrect.'
    Check (Test-Path -LiteralPath (Join-Path $shortcutDir 'LOLPing Lite.lnk')) 'Application Start Menu shortcut is missing.'
    Check (Test-Path -LiteralPath (Join-Path $shortcutDir 'Uninstall.lnk')) 'Uninstaller Start Menu shortcut is missing.'
    Check (-not (Has-RunValue)) 'Installation unexpectedly enabled autostart.'
    $checks.Add('Silent per-user installation, version metadata, shortcuts and disabled-by-default autostart')

    $manifest = Get-Content -LiteralPath (Join-Path $installDir 'checksums.json') -Raw -Encoding utf8 | ConvertFrom-Json
    foreach ($property in $manifest.files.PSObject.Properties) {
        $file = Join-Path $installDir $property.Name
        Check ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -eq $property.Value) "Installed payload hash mismatch: $($property.Name)"
    }
    $checks.Add('Every installed payload file matches the staged SHA-256 inventory')

    # Check the loader's actual module path, not merely whether a DLL happens to
    # exist on this machine. This prevents a globally installed runtime from
    # masking a broken portable/installer layout on developer and CI hosts.
    $probe = [Diagnostics.Process]::new()
    $probe.StartInfo.FileName = $exe
    $probe.StartInfo.UseShellExecute = $false
    $probe.StartInfo.CreateNoWindow = $true
    $probe.StartInfo.RedirectStandardInput = $true
    $probe.StartInfo.RedirectStandardOutput = $true
    $probe.StartInfo.RedirectStandardError = $true
    foreach ($arg in @('--headless-test', '--root', $installDir, '--data-dir', (Join-Path $sandbox 'runtime-probe'))) { $probe.StartInfo.ArgumentList.Add($arg) }
    try {
        Check ($probe.Start()) 'Installed native application could not start.'
        $stdout = $probe.StandardOutput.ReadToEndAsync()
        $stderr = $probe.StandardError.ReadToEndAsync()
        Wait-Until {
            $probe.Refresh()
            Check (-not $probe.HasExited) 'Installed application exited before its runtime could be inspected.'
            $loaded = @($probe.Modules | Where-Object { $_.ModuleName -ieq 'vcruntime140.dll' })
            if ($loaded.Count -eq 0) { return $false }
            Check ([IO.Path]::GetFullPath($loaded[0].FileName) -ieq (Join-Path $installDir 'vcruntime140.dll')) 'Application loaded the global VC runtime instead of its bundled DLL.'
            return $true
        } 'Installed application did not load its bundled runtime.'
        $probe.StandardInput.Close()
        Check ($probe.WaitForExit(10000)) 'Installed application did not exit after its command pipe closed.'
        Check ($probe.ExitCode -eq 0) 'Installed runtime probe exited unsuccessfully.'
    } finally {
        if (-not $probe.HasExited) { $probe.Kill($true); $probe.WaitForExit(5000) | Out-Null }
        $probe.Dispose()
    }
    $checks.Add('Installed application starts and loads VCRUNTIME140.dll from its own installation directory')

    $before = (Get-FileHash -LiteralPath $exe).Hash
    Run-Setup $installer @('/S', "/D=$installDir")
    Check ((Get-FileHash -LiteralPath $exe).Hash -eq $before) 'Same-version upgrade changed the executable unexpectedly.'
    Check ((Get-Content -LiteralPath $marker -Raw -Encoding utf8) -eq $markerText) 'Upgrade modified the user-data marker.'
    $checks.Add('Same-version silent upgrade preserves application files and user data')

    if (-not (Test-Path -LiteralPath $runKey)) { New-Item -Path $runKey -Force | Out-Null }
    New-ItemProperty -LiteralPath $runKey -Name 'LOLPing Lite' -Value ('"' + $exe + '" --background') -PropertyType String -Force | Out-Null
    Run-Setup (Join-Path $installDir 'Uninstall.exe') @('/S')
    Wait-Until { -not (Test-Path -LiteralPath $installDir) -and -not (Test-Path -LiteralPath $uninstallKey) } 'Uninstaller did not finish removing its files and registration.'
    Check (-not (Test-Path -LiteralPath $productKey)) 'Product registry key remained after uninstall.'
    Check (-not (Test-Path -LiteralPath $shortcutDir)) 'Start Menu shortcuts remained after uninstall.'
    Check (-not (Has-RunValue)) 'Autostart registry entry remained after uninstall.'
    Check ((Get-Content -LiteralPath $marker -Raw -Encoding utf8) -eq $markerText) 'Uninstall removed user data.'
    $installed = $false
    $checks.Add('Silent uninstall removes packaged files, shortcuts, registration and startup entry while preserving user data')
} catch {
    $failure = $_.Exception.Message.Replace($repo, '<repository>').Replace($env:USERPROFILE, '<user-profile>')
} finally {
    if ($installed -and (Test-Path -LiteralPath (Join-Path $installDir 'Uninstall.exe'))) {
        try { Run-Setup (Join-Path $installDir 'Uninstall.exe') @('/S') } catch { Write-Warning 'Test cleanup could not run the uninstaller; generated installation is retained for inspection.' }
    }
    if (Has-RunValue) { Remove-ItemProperty -LiteralPath $runKey -Name 'LOLPing Lite' -ErrorAction SilentlyContinue }
    if (Test-Path -LiteralPath $marker) { Remove-Item -LiteralPath $marker }
    # Only remove the empty data directory created above, never user contents.
    foreach ($dir in $createdDataDirs) {
        if ((Test-Path -LiteralPath $dir) -and @(Get-ChildItem -LiteralPath $dir -Force).Count -eq 0) { Remove-Item -LiteralPath $dir }
    }
}
$report = @{ version = $version; target = 'windows-x64'; passed = ($null -eq $failure); checks = $checks.ToArray(); error = $failure; existingInstallationDetected = $false; generatedTestDirectory = ".test-data/$testId" }
Write-Report $report
$report | ConvertTo-Json -Depth 8
if ($failure) { throw $failure }
