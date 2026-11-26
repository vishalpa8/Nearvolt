# Points this terminal at the project toolchain in .toolchain\. Affects only the
# current terminal session.
#
#   . .\tools\env.ps1

$root = Split-Path -Parent $PSScriptRoot
$tc = Join-Path $root '.toolchain'

$env:RUSTUP_HOME = "$tc\rustup"
$env:CARGO_HOME = "$tc\cargo"
$env:PUB_CACHE = "$tc\pub-cache"
$env:GRADLE_USER_HOME = "$tc\gradle"
$env:ANDROID_HOME = "$tc\android-sdk"
$env:ANDROID_SDK_ROOT = $env:ANDROID_HOME
$env:ANDROID_NDK_HOME = "$env:ANDROID_HOME\ndk\28.2.13676358"
$env:FLUTTER_SUPPRESS_ANALYTICS = 'true'
# Corporate networks often block certificate revocation servers, which makes every
# crates.io download fail. Certificates are still verified; only that lookup is skipped.
$env:CARGO_HTTP_CHECK_REVOKE = 'false'

# Java tools trust the Windows certificate store, which includes any certificate a
# corporate network inspects traffic with.
$java = Get-Command java -ErrorAction SilentlyContinue
if ($java) {
    $env:JAVA_HOME = Split-Path -Parent (Split-Path -Parent $java.Source)
}
$javaTrust = '-Djavax.net.ssl.trustStoreType=Windows-ROOT'
$env:SDKMANAGER_OPTS = $javaTrust
$env:GRADLE_OPTS = $javaTrust

$paths = "$tc\cargo\bin", "$tc\llvm-mingw\bin", "$tc\flutter\bin", "$tc\android-sdk\platform-tools"
$env:PATH = (($paths + ($env:PATH -split ';')) | Where-Object { $_ } | Select-Object -Unique) -join ';'
