# Nearvolt security

Nearvolt has no server and no accounts, so every security decision happens on the
two devices.

## Threats

| Threat | Defense |
|---|---|
| Someone nearby intercepts or reads the transfer | TLS 1.3 encryption on every byte, over Wi-Fi and USB alike |
| Someone nearby impersonates the receiver | With a QR code or NFC tap, the sender only accepts the certificate in the ticket. From the Nearby list or over USB, both screens show a 4-digit code bound to both certificates, and the receiver's user accepts only when they match |
| A device in the middle tries to make the two codes match | The sender commits to its random nonce before seeing the receiver's, so the codes can't be steered; each attempt has a 1 in 10,000 chance and needs the user to accept |
| Someone nearby pushes files to you | The receiver's user accepts each connection, comparing the code when there is one. Only a phone held against yours connects without asking. While one phone is connected, every other is turned away |
| Brute-forcing the pairing secret | 128-bit secret, constant-time comparison, failed handshakes are ignored and limited to 32 at once, so they can't end Receive |
| A malicious sender writes outside the download folder | Every path is cleaned: `..`, absolute paths, drive letters, `\`, reserved names and control characters are rejected or replaced |
| A malicious sender fills the storage | The offer's total size is checked against free space before anything is written |
| A malicious sender sends malformed data | Batch, file index, offset and length are checked against the offer; messages have size limits; a file is saved only after the bytes on disk match the hash |
| Two offered names map to the same file (`A.txt` and `a.txt`) | Case-insensitive collision detection renames duplicates |
| Received files executing automatically | Files are never opened automatically |
| A fake update | See [Updates](#updates) |
| Memory-safety bugs | Engine in Rust; Kotlin and Dart are memory-safe too |

## Pairing

- Each device creates an Ed25519 identity on first launch and keeps it in the
  app's private storage, which other apps can't read.
- A private ticket carries the receiver's fingerprint and a fresh 128-bit secret.
  The QR code and the NFC tap each get their own secret. The QR secret proves the
  sender saw the screen, and the receiver's user still accepts. The tap secret
  is served only over NFC, at a few centimetres, so holding the phones together
  is the consent and there's no prompt.
- The public ticket advertised over the local network and Bluetooth has no
  secret, so connections started from it always go through the code. It also
  leaves out the Wi-Fi Direct group's name and password: everyone on the network
  and in Bluetooth range can read it, and with the password anyone could join
  the receiver's own network. Phones on different networks connect with the QR
  code or a tap, which carry the group details. Tickets leave out mobile-data
  and VPN addresses too.

## Listening

The receiver listens only while the Receive page is open, on a random port, and
the network listener, Bluetooth advertisement, NFC card and Wi-Fi Direct group all
stop when it closes. Every receive session creates a new secret. Streams arriving
over a USB cable are dropped unless Receive is open.

## Permissions

- Bluetooth and Wi-Fi Direct are requested with `neverForLocation`; location is
  only requested on Android 11 and 12, where the system requires it for scans.
- Sending uses the system file picker, so the app has no storage permission.
- Received files go to `Download/Nearvolt`, which needs no permission on
  Android 11 and later.
- The QR scanner is Google's code scanner, so the app has no camera permission.

## Updates

- Release APKs are signed with the Nearvolt release key, which never enters the
  repository. GitHub Actions gets it from encrypted repository secrets, and the
  release workflow refuses to publish an APK whose certificate isn't exactly the
  release certificate (SHA-256 `9ffe2712…8fc5ba`).
- The app contacts GitHub only when the user taps Check for updates. It
  downloads `update.json` and the APK for its ABI only from this repository's
  GitHub Releases over HTTPS, and checks the APK's SHA-256 before installing it.
- Android itself refuses to install an update signed with any other key.
- An update is never installed while connected to another phone.

## Before a public launch

- `cargo audit` and `cargo deny` clean.
- Fuzz the ticket, frame and control-message parsers with `cargo fuzz`.
- External security review.
