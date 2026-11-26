# Downloads everything Nearvolt needs into .toolchain\ inside the project.
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

# Rust for Android, and the tools that build and bind it.
Step 'Rust Android targets and build tools'
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
foreach ($tool in @(@('cargo-ndk', '4.1.2'), @('flutter_rust_bridge_codegen', '2.13.0'))) {
    $name, $version = $tool
    $installed = cargo install --list | Select-String -SimpleMatch "$name v$version"
    if (-not $installed) {
        cargo install $name --version $version --locked
        if ($LASTEXITCODE -ne 0) { throw "Installing $name failed" }
    }
}

# Android SDK and NDK, project-local.
$sdkManager = "$env:ANDROID_HOME\cmdline-tools\latest\bin\sdkmanager.bat"
if (-not (Test-Path $sdkManager)) {
    Step 'Downloading Android command-line tools'
    [xml]$repo = (Invoke-WebRequest -UseBasicParsing 'https://dl.google.com/android/repository/repository2-3.xml').Content
    $package = $repo.SelectNodes('//remotePackage') | Where-Object { $_.path -eq 'cmdline-tools;latest' } | Select-Object -First 1
    $archive = $package.archives.archive | Where-Object { $_.'host-os' -eq 'windows' } | Select-Object -First 1
    $zip = Join-Path $tc 'cmdline-tools.zip'
    Invoke-WebRequest -UseBasicParsing "https://dl.google.com/android/repository/$($archive.complete.url)" -OutFile $zip
    $staging = Join-Path $tc 'cmdline-tools-staging'
    tar -xf $zip -C (New-Item -ItemType Directory -Force $staging)
    New-Item -ItemType Directory -Force "$env:ANDROID_HOME\cmdline-tools" | Out-Null
    Move-Item "$staging\cmdline-tools" "$env:ANDROID_HOME\cmdline-tools\latest"
    Remove-Item $zip, $staging -Recurse
}
Step 'Installing Android SDK packages'
$packages = 'platform-tools', 'platforms;android-36', 'build-tools;36.0.0', 'ndk;28.2.13676358'
(1..20 | ForEach-Object { 'y' }) | & $sdkManager --licenses | Out-Null
& $sdkManager --install @packages
if ($LASTEXITCODE -ne 0) { throw 'Installing Android SDK packages failed' }

# Gradle trusts the Windows certificate store too (see env.ps1).
$gradleProps = "$env:GRADLE_USER_HOME\gradle.properties"
if (-not (Test-Path $gradleProps)) {
    New-Item -ItemType Directory -Force $env:GRADLE_USER_HOME | Out-Null
    Set-Content $gradleProps 'systemProp.javax.net.ssl.trustStoreType=Windows-ROOT'
}

Step 'Done. In any new terminal, run:  . .\tools\env.ps1'
