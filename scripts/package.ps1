param(
    [ValidateSet("default", "all", "nsis", "wix", "dmg", "deb", "appimage", "pacman")]
    [string]$Format = "default"
)

$ErrorActionPreference = "Stop"

if (-not (Get-Command cargo-packager -ErrorAction SilentlyContinue)) {
    throw "cargo-packager is not installed. Run: cargo install cargo-packager --locked --version 0.11.8"
}

cargo packager --release --formats $Format
if ($LASTEXITCODE -ne 0) {
    throw "cargo-packager exited with code $LASTEXITCODE"
}

Write-Host "Packages were written to target/release."
