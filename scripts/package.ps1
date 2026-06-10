$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$version = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"').Matches.Groups[1].Value
$dist = Join-Path $root "dist"
$stage = Join-Path $dist "wintfy-rs-v$version-windows-x64"
$zip = "$stage.zip"

cargo build --release --manifest-path (Join-Path $root "Cargo.toml")

if (Test-Path $stage) {
    Remove-Item -LiteralPath $stage -Recurse -Force
}
New-Item -ItemType Directory -Path $stage | Out-Null

Copy-Item -LiteralPath (Join-Path $root "target\release\wintfy-rs.exe") -Destination (Join-Path $stage "wintfy-rs.exe")
Copy-Item -LiteralPath (Join-Path $root "config.example.toml") -Destination (Join-Path $stage "config.example.toml")
Copy-Item -LiteralPath (Join-Path $root "README.md") -Destination (Join-Path $stage "README.md")
Copy-Item -LiteralPath (Join-Path $root "LICENSE") -Destination (Join-Path $stage "LICENSE")

if (Test-Path $zip) {
    Remove-Item -LiteralPath $zip -Force
}
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
Write-Host "Created $zip"
