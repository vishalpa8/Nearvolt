# Nearvolt security

Nearvolt has no server and no accounts, so every security decision happens on the
two devices.

## Threats

| Threat | Defense |
|---|---|
| Someone nearby intercepts or reads the transfer | TLS 1.3 encryption on every byte; the receiver's key is pinned through the ticket |
| Someone nearby impersonates the receiver | The sender only accepts the certificate whose fingerprint is in the ticket it scanned or tapped |
| Someone nearby pushes files to you | The sender must present the ticket's one-time secret, and the receiver approves every offer |
| A malicious sender writes outside the download folder | Every path is cleaned: `..`, absolute paths, drive letters, `\`, reserved Windows names and control characters are rejected or replaced |
| A malicious sender fills the disk | The offer's total size is checked against free space before anything is written |
| A malicious sender sends malformed frames | File index, chunk index and length are checked against the offer; messages have size limits; the hash is verified before writing |
| Two offered names map to the same file (`A.txt` and `a.txt` on Windows) | Case-insensitive collision detection renames duplicates |
| Brute-forcing the pairing secret | 128-bit secret, constant-time comparison, receiver stops after repeated failures |
| Received files executing automatically | Files are never opened automatically |
| Memory-safety bugs | Core engine in Rust |

## Pairing

- Each device creates an Ed25519 identity on first launch. On phones the key will
  live in the Android Keystore or Apple Keychain.
- A ticket carries the receiver's fingerprint and a fresh 128-bit secret. It's
  transferred by NFC tap or QR scan, which an attacker can't intercept from a
  distance.
- Bluetooth-only pairing (planned) shows the same 4-digit code on both screens,
  derived from both fingerprints, for the users to confirm.

## Listening

The receiver listens only while the receive screen is open, and only for the
current ticket. A new receive session creates a new secret.

## Release checklist

- `cargo audit` and `cargo deny` clean.
- Fuzz the frame and control-message parsers with `cargo fuzz`.
- External security review before public launch.
- No analytics, no telemetry, no network calls besides the peer link.
