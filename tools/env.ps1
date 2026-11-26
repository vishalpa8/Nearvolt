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
$env:FLUTTER_SUPPRESS_ANALYTICS = 'true'
# Corporate networks often block certificate revocation servers, which makes every
# crates.io download fail. Certificates are still verified; only that lookup is skipped.
$env:CARGO_HTTP_CHECK_REVOKE = 'false'

$paths = "$tc\cargo\bin", "$tc\llvm-mingw\bin", "$tc\flutter\bin"
$env:PATH = (($paths + ($env:PATH -split ';')) | Where-Object { $_ } | Select-Object -Unique) -join ';'
