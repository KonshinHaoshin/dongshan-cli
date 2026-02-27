param(
  [string]$Version = "",
  [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Version)) {
  $Version = (Get-Content Cargo.toml | Select-String '^version\s*=\s*"(.*)"' | ForEach-Object { $_.Matches[0].Groups[1].Value } | Select-Object -First 1)
}

if ([string]::IsNullOrWhiteSpace($Version)) {
  throw "Failed to detect version from Cargo.toml"
}

Write-Host "Building dongshan ($Profile) version $Version ..."
cargo build --profile $Profile

$exePath = "target\$Profile\dongshan.exe"
if (!(Test-Path $exePath)) {
  throw "Binary not found: $exePath"
}

$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (!(Test-Path $iscc)) {
  throw "Inno Setup not found. Install Inno Setup 6 first."
}

New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item $exePath "dist\dongshan.exe" -Force
Compress-Archive -Path "dist\dongshan.exe" -DestinationPath "dist\dongshan-windows-x86_64.zip" -Force

& $iscc "packaging\windows\dongshan.iss" "/DMyAppVersion=$Version" "/DMyAppExe=$exePath" "/Odist" "/Fdongshan-setup-windows-x86_64"

Get-ChildItem dist
Write-Host "Done. Installer and zip are in dist/."
