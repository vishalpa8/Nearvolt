# Proxima

Move files directly between nearby devices at the full speed of the link. No
server, no account, no internet. Every byte is verified, and interrupted transfers
resume from the last verified chunk.

## Status

- **Core engine** (`core/`): pairing tickets, TLS 1.3 transport with pinned
  certificates, parallel connections, chunk verification, crash-safe journal,
  automatic reconnect and resume. Tested with link cuts and killed apps.
- **Test tool** (`tools/bench/`): send and receive between computers, generate test
  data, and run local stress tests.
- **App** (`app/`): Flutter UI with the four screens from
  [docs/design.md](docs/design.md). It runs on a simulated engine, marked
  "Preview" in the app, until the Rust engine is connected through
  `flutter_rust_bridge`.

See [docs/architecture.md](docs/architecture.md) for the design and measured
results.

## Setup on Windows

Everything installs into `.toolchain\` inside this folder. Nothing is installed
system-wide.

```powershell
powershell -ExecutionPolicy Bypass -File tools\setup.ps1   # Rust, LLVM-MinGW, Flutter
. .\tools\env.ps1                                           # use them in this terminal
```

## Try the engine

```powershell
cargo build --release -p proxima-bench
$b = ".\target\release\proxima-bench.exe"

& $b gen .bench\data                  # 2.5 GB of large files and photos
& $b local .bench\data                # send to this PC, then verify every byte
& $b local .bench\data --drop-link-at-mb 800 --kill-at-mb 1600   # with failures
```

Between two computers on the same network:

```powershell
& $b receive --dir received           # prints a ticket
& $b send <ticket> <files or folders>
```

## Try the app

```powershell
cd app
flutter test
flutter build web --release --no-web-resources-cdn   # then serve build\web
```

Open `#/nearby`, `#/pick`, `#/receive`, `#/transfer` or `#/done` to jump straight
to a screen with sample data.

## Docs

- [Architecture](docs/architecture.md)
- [Wire protocol](docs/protocol.md)
- [Security](docs/security.md)
- [Design system](docs/design.md)
