$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $ProjectRoot

$Version = (Select-String -Path "Cargo.toml" -Pattern '^version = "(.+)"$').Matches.Groups[1].Value
$PackageName = "FastAccess-$Version-windows-x64"
$DistDirectory = Join-Path $ProjectRoot "dist"
$StageDirectory = Join-Path $DistDirectory $PackageName

cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release

New-Item -ItemType Directory -Force -Path $StageDirectory | Out-Null
Copy-Item "target\release\fastaccess.exe" $StageDirectory
Copy-Item "README.md" $StageDirectory
Copy-Item "LICENSE" $StageDirectory

$ArchivePath = Join-Path $DistDirectory "$PackageName.zip"
if (Test-Path $ArchivePath) {
    Remove-Item $ArchivePath
}
Compress-Archive -Path "$StageDirectory\*" -DestinationPath $ArchivePath

Write-Host "Release package: $ArchivePath"

