# Build the two Windows archives natively (MSVC toolchain) into dist\.
#
# Produces, each unpacking to a link-resolver\ folder:
#   link-resolver-gui-windows-x64.zip   desktop app + CLI (Windows 10/11)
#   link-resolver-windows-x64.zip       terminal-only edition
# plus unpacked copies and SHA256SUMS. Existing Windows entries in dist\ are replaced; Linux
# archives already there are left alone. This is the Windows half of packaging/package-dist.sh
# and keeps the same layout, so the release workflow can merge the two.
#
# Requirements: Rust (rustup, stable, x86_64-pc-windows-msvc) and PowerShell 5.1 or 7+.
# Usage: powershell -ExecutionPolicy Bypass -File packaging\package-windows.ps1

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$dist = Join-Path $root 'dist'
$stage = Join-Path $root 'target\dist-staging'
$out = Join-Path $root 'target\release'
Set-Location $root

Write-Host '== Building Windows (GUI + CLI) =='
cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

Write-Host '== Checking build outputs =='
# The PE optional header records which subsystem an executable runs under: 2 = GUI
# (no console window), 3 = console. The same check package-dist.sh makes with objdump.
function Get-Subsystem([string] $path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    $peOffset = [System.BitConverter]::ToInt32($bytes, 0x3C)
    if ([System.Text.Encoding]::ASCII.GetString($bytes, $peOffset, 4) -ne "PE`0`0") {
        throw "$path is not a Windows executable"
    }
    $optionalHeader = $peOffset + 4 + 20
    return [System.BitConverter]::ToUInt16($bytes, $optionalHeader + 68)
}
if ((Get-Subsystem (Join-Path $out 'link-resolver.exe')) -ne 2) {
    throw 'link-resolver.exe is not a GUI-subsystem executable'
}
if ((Get-Subsystem (Join-Path $out 'link-resolver-cli.exe')) -ne 3) {
    throw 'link-resolver-cli.exe is not a console executable'
}
# A real run of the console binary, touching no network: it must report the version that
# Cargo.toml carries (the release version once packaging/stamp-version.py has run).
$expected = (Select-String -Path 'Cargo.toml' -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$reported = (& (Join-Path $out 'link-resolver-cli.exe') --version) -join ' '
if ($LASTEXITCODE -ne 0 -or $reported -ne "Link Resolver $expected") {
    throw "link-resolver-cli.exe --version printed '$reported', expected 'Link Resolver $expected'"
}
Write-Host "link-resolver-cli.exe --version OK ($reported)"

Write-Host '== Staging =='
foreach ($folder in 'gui-windows-x64', 'windows-x64') {
    Remove-Item -Recurse -Force (Join-Path $dist $folder) -ErrorAction SilentlyContinue
}
Remove-Item -Force (Join-Path $dist 'link-resolver-gui-windows-x64.zip') -ErrorAction SilentlyContinue
Remove-Item -Force (Join-Path $dist 'link-resolver-windows-x64.zip') -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $dist, $stage | Out-Null

# Stage-Folder <folder> <quickstart> @{ source = name; ... }
function Stage-Folder([string] $folder, [string] $quickstart, [hashtable] $binaries) {
    $target = Join-Path $dist $folder
    New-Item -ItemType Directory -Force $target | Out-Null
    Copy-Item $quickstart (Join-Path $target 'QUICKSTART.txt')
    Copy-Item 'examples\urls.txt' (Join-Path $target 'urls.txt')
    Copy-Item 'examples\links.csv' (Join-Path $target 'links.csv')
    foreach ($source in $binaries.Keys) {
        Copy-Item (Join-Path $out $source) (Join-Path $target $binaries[$source])
    }
}
Stage-Folder 'gui-windows-x64' 'QUICKSTART.txt' @{
    'link-resolver.exe' = 'link-resolver.exe'
    'link-resolver-cli.exe' = 'link-resolver-cli.exe'
}
Stage-Folder 'windows-x64' 'packaging\QUICKSTART-terminal.txt' @{
    'link-resolver-cli.exe' = 'link-resolver.exe'
}

# New-Archive <folder> <zip name>: the zip unpacks to link-resolver\.
function New-Archive([string] $folder, [string] $zipName) {
    $packed = Join-Path $stage 'link-resolver'
    Remove-Item -Recurse -Force $packed -ErrorAction SilentlyContinue
    Copy-Item -Recurse (Join-Path $dist $folder) $packed
    Compress-Archive -Path $packed -DestinationPath (Join-Path $dist $zipName) -CompressionLevel Optimal
}
New-Archive 'gui-windows-x64' 'link-resolver-gui-windows-x64.zip'
New-Archive 'windows-x64' 'link-resolver-windows-x64.zip'
Remove-Item -Recurse -Force $stage

# Same line format as sha256sum, with forward slashes, so the two halves merge cleanly.
$sums = Get-ChildItem $dist -Recurse -File -Depth 1 |
    Where-Object { $_.Name -like 'link-resolver*' -or $_.Extension -in '.zip', '.gz' } |
    ForEach-Object {
        $relative = $_.FullName.Substring($dist.Length + 1).Replace('\', '/')
        '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $relative
    } | Sort-Object { $_.Substring(66) }
[System.IO.File]::WriteAllText((Join-Path $dist 'SHA256SUMS'), (($sums -join "`n") + "`n"))
Write-Host '== Done =='
$sums
