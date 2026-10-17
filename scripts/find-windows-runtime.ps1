$ErrorActionPreference = 'Stop'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) { throw 'Visual Studio Installer vswhere.exe is required to locate licensed VC redistributables.' }
$installations = @((& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json) | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0 -or $installations.Count -ne 1) { throw 'No supported, non-preview Visual Studio C++ toolchain was found.' }
$installation = $installations[0]
$major = ([version]$installation.installationVersion).Major
if ($major -notin @(17, 18)) { throw "Review the runtime license for unsupported Visual Studio major version $major before packaging." }
$redistRoot = Join-Path $installation.installationPath 'VC/Redist/MSVC'
$versions = Get-ChildItem -LiteralPath $redistRoot -Directory | Where-Object { $_.Name -match '^14\.\d+\.\d+$' } | Sort-Object { [version]$_.Name } -Descending
$crt = $null
foreach ($version in $versions) {
    $x64 = Join-Path $version.FullName 'x64'
    if (Test-Path -LiteralPath $x64) {
        $crt = Get-ChildItem -LiteralPath $x64 -Directory | Where-Object { $_.Name -match '^Microsoft\.VC\d+\.CRT$' } | Select-Object -First 1
        if ($crt) { break }
    }
}
if (-not $crt) { throw 'No redistributable x64 CRT was found. Do not copy DLLs from System32 or debug_nonredist.' }
$libraries = @()
foreach ($file in Get-ChildItem -LiteralPath $crt.FullName -File -Filter '*.dll') {
    $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation(?:,|$)') {
        throw "The Visual C++ runtime does not have a valid Microsoft signature: $($file.Name)"
    }
    $fileVersion = $file.VersionInfo.FileVersion
    if ($fileVersion -notmatch '^14\.') { throw "Unexpected VC runtime version: $fileVersion" }
    $libraries += @{ name = $file.Name; path = $file.FullName; source = [IO.Path]::GetRelativePath($installation.installationPath, $file.FullName).Replace('\', '/'); fileVersion = $fileVersion; signer = $signature.SignerCertificate.Subject; signature = 'Valid' }
}
@{ visualStudioMajor = $major; redistributableVersion = $version.Name; libraries = $libraries } | ConvertTo-Json -Depth 5
