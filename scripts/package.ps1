param(
    [ValidateSet("default", "all", "nsis", "wix", "dmg", "deb", "appimage", "pacman")]
    [string]$Format = "default"
)

$ErrorActionPreference = "Stop"
$ExpectedPackagerVersion = "0.11.8"

if (-not (Get-Command cargo-packager -ErrorAction SilentlyContinue)) {
    throw "cargo-packager is not installed. Run: cargo install cargo-packager --locked --version '=$ExpectedPackagerVersion'"
}

$actualVersion = cargo-packager --version
if ($LASTEXITCODE -ne 0) {
    throw "cargo-packager --version exited with code $LASTEXITCODE"
}
$expectedVersionOutput = "cargo-packager $ExpectedPackagerVersion"
if ($actualVersion -cne $expectedVersionOutput) {
    throw "Expected $expectedVersionOutput, found $actualVersion. Reinstall with: cargo install cargo-packager --locked --version '=$ExpectedPackagerVersion'"
}

cargo-packager --release --formats $Format
if ($LASTEXITCODE -ne 0) {
    throw "cargo-packager exited with code $LASTEXITCODE"
}

Write-Host "Packages were written to target/release."
