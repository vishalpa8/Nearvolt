# Nearvolt

Move files directly between nearby Android phones at the full speed of the link.
No server, no account, no internet. Every byte is verified, and interrupted
transfers resume from the last verified chunk.

Connect from the Nearby list (same Wi-Fi or Bluetooth, then Wi-Fi Direct), by
scanning a QR code, by tapping phones together, or with a USB cable.

## Setup on Windows

Everything installs into `.toolchain\` inside this folder. Nothing is installed
system-wide.

```powershell
powershell -ExecutionPolicy Bypass -File tools\setup.ps1   # Rust, Flutter, Android SDK and NDK
. .\tools\env.ps1                                           # use them in this terminal
```

## Build and run

```powershell
cd app
$env:NEARVOLT_ABIS = 'arm64-v8a'      # optional: build the engine for one ABI only
flutter run                           # on a phone connected with USB debugging
flutter build apk --release --split-per-abi
```

Gradle builds the Rust engine (`ffi/`) with `cargo-ndk` before every build. After
changing the engine's API in `ffi/src/api.rs`, run `flutter_rust_bridge_codegen
generate` in `app/`; after changing `app/pigeons/platform.dart`, run `dart run
pigeon --input pigeons/platform.dart`.

## Test

```powershell
cargo test --workspace
cd app; flutter test
```

[docs/testing.md](docs/testing.md) has the checklist for two real phones.

## Release

Releases are built, signed and published by GitHub Actions. Installed apps find
them and update themselves.

1. Once: add the four repository secrets listed in the signing folder's
   `github-secrets.txt` (`NEARVOLT_KEYSTORE_BASE64`, `NEARVOLT_KEYSTORE_PASSWORD`,
   `NEARVOLT_KEY_ALIAS`, `NEARVOLT_KEY_PASSWORD`), and keep an offline backup of
   the key. Without it, installed apps can never be updated again.
2. Tag and push:

   ```powershell
   git tag -a v1.2.3 -m "What changed"
   git push origin v1.2.3
   ```

The tag sets the version (`1.2.3`, version code `10203`), and the tag message
becomes the release notes. Local release builds sign with the same key through
`app/android/key.properties`, which Git ignores.

## Engine test tool

```powershell
cargo build --release -p nearvolt-bench
$b = ".\target\release\nearvolt-bench.exe"
& $b gen .bench\data                  # 2.5 GB of large files and photos
& $b local .bench\data --drop-link-at-mb 800 --kill-at-mb 1600   # send to this PC with failures
& $b receive --dir received           # between two computers: prints a ticket
& $b send <ticket> <files or folders>
```

## Docs

- [Architecture](docs/architecture.md)
- [Wire protocol](docs/protocol.md)
- [Security](docs/security.md)
- [Design system](docs/design.md)
- [Testing](docs/testing.md)
