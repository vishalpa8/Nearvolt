# Nearvolt

Move files directly between nearby Android phones at the full speed of the link.
No server, no account, no internet. Every file is verified on disk, and
interrupted transfers resume where they stopped.

Connect by tapping phones together, by scanning a QR code (both then move the files
over a private Wi-Fi Direct link), or with a USB cable. The phones
stay connected until either one taps Disconnect, and the sender can keep adding
files.

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

Every push to `main` that changes the app is tested, built, signed and published
as a GitHub release by Actions. Pushes that only touch docs are skipped, and
Actions › Release › Run workflow builds one on demand.

- The version is `major.minor` from `app/pubspec.yaml` plus the commit count on
  `main` (for example `0.1.27`); the version code is 100 plus that count.
- The release notes list the commit subjects since the previous release.
- In the app, **Check for updates** on the home page downloads the new version
  and installs it. The first time, Android asks to allow installs from
  Nearvolt; after that, Android 12 and later update without a prompt. On
  Samsung phones, Auto Blocker must be off while updating. A version is
  downloaded once, and the APK is deleted after it's installed.

The signing key lives in four repository secrets (`NEARVOLT_KEYSTORE_BASE64`,
`NEARVOLT_KEYSTORE_PASSWORD`, `NEARVOLT_KEY_ALIAS`, `NEARVOLT_KEY_PASSWORD`).
Keep an offline backup of it: without it, installed apps can never be updated
again. Local release builds sign with the same key through
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
