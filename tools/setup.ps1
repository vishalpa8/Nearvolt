# Downloads everything Proxima needs into .toolchain\ inside the project.
# Nothing is installed system-wide and no system settings are changed.
#
#   powershell -ExecutionPolicy Bypass -File tools\setup.ps1

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$root = Split-Path -Parent $PSScriptRoot
$tc = Join-Path $root '.toolchain'
New-Item -ItemType Directory -Force -Path $tc | Out-Null

. (Join-Path $PSScriptRoot 'env.ps1')

function Step($text) { Write-Host "`n==> $text" -ForegroundColor Cyan }

# Rust. The gnullvm host pairs with LLVM-MinGW, so no Visual Studio is required.
if (-not (Test-Path "$env:CARGO_HOME\bin\cargo.exe")) {
    Step 'Installing Rust into .toolchain'
    $init = Join-Path $tc 'rustup-init.exe'
    Invoke-WebRequest -UseBasicParsing -Uri 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe' -OutFile $init
    & $init -y --no-modify-path --profile minimal --default-host x86_64-pc-windows-gnullvm --default-toolchain stable
    if ($LASTEXITCODE -ne 0) { throw 'rustup-init failed' }
    Remove-Item $init
    rustup component add rustfmt clippy
} else {
    Step 'Rust already installed'
}

# LLVM-MinGW: portable C compiler and linker used by Rust crates with C code.
if (-not (Test-Path "$tc\llvm-mingw\bin\clang.exe")) {
    Step 'Downloading LLVM-MinGW'
    $release = Invoke-RestMethod -Uri 'https://api.github.com/repos/mstorsjo/llvm-mingw/releases/latest'
    $asset = $release.assets | Where-Object { $_.name -match '^llvm-mingw-.*-ucrt-x86_64\.zip$' } | Select-Object -First 1
    if (-not $asset) { throw 'Could not find the LLVM-MinGW download' }
    $zip = Join-Path $tc $asset.name
    Invoke-WebRequest -UseBasicParsing -Uri $asset.browser_download_url -OutFile $zip
    tar -xf $zip -C $tc
    if ($LASTEXITCODE -ne 0) { throw 'Extracting LLVM-MinGW failed' }
    Remove-Item $zip
    Rename-Item (Join-Path $tc ($asset.name -replace '\.zip$', '')) 'llvm-mingw'
} else {
    Step 'LLVM-MinGW already installed'
}

# Flutter (stable channel).
if (-not (Test-Path "$tc\flutter\bin\flutter.bat")) {
    Step 'Downloading Flutter'
    git clone -b stable --single-branch https://github.com/flutter/flutter.git "$tc\flutter"
    if ($LASTEXITCODE -ne 0) { throw 'Cloning Flutter failed' }
} else {
    Step 'Flutter already installed'
}

Step 'Preparing Flutter'
flutter --disable-analytics | Out-Null
flutter config --no-analytics | Out-Null
flutter --version

Step 'Done. In any new terminal, run:  . .\tools\env.ps1'
