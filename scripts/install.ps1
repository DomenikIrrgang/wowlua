# Installs the latest (or a given) wowlua release on Windows.
#   irm https://github.com/DomenikIrrgang/wowlua/releases/latest/download/install.ps1 | iex
#   $env:WOWLUA_VERSION = "v0.1.0"; ./install.ps1
$ErrorActionPreference = "Stop"

$repo = "DomenikIrrgang/wowlua"
$version = if ($env:WOWLUA_VERSION) { $env:WOWLUA_VERSION } else { "latest" }
$installDir = if ($env:WOWLUA_INSTALL_DIR) { $env:WOWLUA_INSTALL_DIR } else { Join-Path $HOME ".wowlua\bin" }
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "aarch64" } else { "x86_64" }
$target = "$arch-pc-windows-msvc"

if ($version -eq "latest") {
    $version = (Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest").tag_name
}
$name = "wowlua-$version-$target"
$url = "https://github.com/$repo/releases/download/$version/$name.zip"

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Write-Host "downloading $url"
    Invoke-WebRequest $url -OutFile "$tmp\$name.zip"
    Invoke-WebRequest "$url.sha256" -OutFile "$tmp\$name.zip.sha256"
    $expected = (Get-Content "$tmp\$name.zip.sha256").Split(" ")[0]
    $actual = (Get-FileHash "$tmp\$name.zip" -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) { throw "checksum mismatch for $name.zip" }
    Expand-Archive "$tmp\$name.zip" -DestinationPath $tmp
    New-Item -ItemType Directory -Force $installDir | Out-Null
    Copy-Item "$tmp\$name\wowlua.exe" $installDir -Force
    Write-Host "installed wowlua $version to $installDir\wowlua.exe"
    if (-not ($env:PATH -split ";" -contains $installDir)) { Write-Host "add $installDir to your PATH" }
}
finally {
    Remove-Item -Recurse -Force $tmp
}
