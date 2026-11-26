# Nearvolt wire protocol, version 1

## Transport

- TLS 1.3 only. ALPN: `nearvolt/1`.
- Over Wi-Fi, every stream is a TCP connection. Over a USB cable, every stream is
  a yamux stream multiplexed on the one cable (see [USB](#usb)). TLS and
  everything above it are identical on both.
- Both sides present a self-signed Ed25519 certificate (the device identity).
  No certificate authorities are used.
- With a **private ticket** (QR code or NFC tap) the sender pins the receiver's
  certificate: the BLAKE3 hash of its DER encoding must equal the fingerprint in
  the ticket. Without one (Nearby list or USB), both devices confirm a 4-digit
  code instead.
- A session uses one **control stream** and several **data streams** (8 by
  default). Parallel streams spread encryption over CPU cores and let the
  kernel's TCP offloads carry the bulk of the work.

Why TCP rather than QUIC: on the development PC, the best QUIC configuration
(quinn, several connections and sockets) reached about 300 MB/s over loopback,
while 8 TCP connections reached 919 MB/s. Kernel TCP also uses less CPU per byte,
which matters for phone battery and heat.

## Ticket

The receiver creates a ticket for each receive session. Text form: `NV1`
followed by unpadded RFC 4648 base32, so it fits the QR alphanumeric mode.

- The **private ticket** carries a one-time secret. It travels only by QR code
  and NFC tap, which can't be picked up from across a room.
- The **public ticket** is the same without the secret. It's advertised over the
  local network (mDNS TXT records) and Bluetooth LE (a GATT characteristic), so
  senders can list the receiver.

Binary layout, all integers big-endian:

| Field | Size |
|---|---|
| version (1) | 1 byte |
| flags: 1 = has secret, 2 = has Wi-Fi Direct group | 1 byte |
| receiver certificate fingerprint (BLAKE3) | 32 bytes |
| one-time secret, if flag 1 | 16 bytes |
| address count (at most 16) | 1 byte |
| each address: family (4 or 6), IP bytes, TCP port | 7 or 19 bytes |
| device name | 1-byte length, then UTF-8 (at most 64 bytes) |
| if flag 2: group SSID, passphrase | each 1-byte length, then UTF-8 |
| if flag 2: group frequency in MHz | 2 bytes |

The ticket is refreshed when the receiver's Wi-Fi Direct group comes up, so it
also carries the group's address. A sender that can't reach any address within
2.5 seconds joins the group (no pop-up on Android 10 and later) and keeps trying
every address.

## Control messages

Each message is a 4-byte little-endian length followed by JSON (at most 64 MiB).

```
sender                                               receiver
  | -- hello {version, secret | commitment, name} ---> |   secret compared in constant time
  | <-- welcome {name, session, nonce?} | reject ----- |   session: random 16-byte ID
  | -- reveal {nonce} (code pairing only) -----------> |   must match the commitment
  |                                                    |   both screens show the code
  | -- offer {chunk_size, files[]} ------------------> |   paths cleaned, space checked, user approves
  | <-- accept {have[]} | reject --------------------- |   have[] = none | complete | partial bitmap
  |                                                    |
  | == data streams: hello {secret?, session}, then chunk frames ==> |
  |                                                    |
  | -- ping (every 500 ms) --------------------------> |
  | <-- progress {files, bytes} (500 ms) ------------- |   verified progress, doubles as heartbeat
  | <-- resend {chunks[]} (rare) --------------------- |   on-disk verification failed
  | <-- complete {files, bytes} ---------------------- |
```

- `offer.files[i]` is `{path, size, modified}`. `path` is relative and uses `/`
  separators. `modified` is milliseconds since the Unix epoch, when known.
- A data stream is accepted only if its session matches the current control
  stream and its certificate matches the sender's.
- Four seconds without a message on the control stream means the link is gone.
  Both sides drop the session; the sender reconnects for up to 60 seconds and
  re-offers.
- If the sender has nothing left to send and verified progress stops for 30
  seconds, it restarts the session so the receiver can report what's missing.

The receiver derives the transfer ID as the BLAKE3 hash of the sender's
certificate fingerprint and the canonical offer. Re-offering the same files after a
reconnect maps to the same ID, which is how resume finds the journal.

## Code pairing

Used when the sender has no secret: a device picked from the Nearby list, or the
USB cable.

1. The sender picks a random 16-byte nonce `Ns` and sends
   `commitment = BLAKE3("nearvolt commitment" ‖ Ns)` in its hello.
2. The receiver replies with its own random nonce `Nr` in the welcome.
3. The sender reveals `Ns`; the receiver checks it against the commitment.
4. Both compute `BLAKE3("nearvolt pairing code" ‖ receiver fingerprint ‖ sender
   fingerprint ‖ Ns ‖ Nr)`, take the first 4 bytes as a little-endian number, and
   show it modulo 10,000 as four digits.

Because the sender commits before it sees `Nr`, a device in the middle can't
choose nonces that make the two codes match; it has a 1 in 10,000 chance per
attempt, and the receiver's user has to accept each one.

## Chunk frame

| Field | Size |
|---|---|
| file index | u32 LE |
| chunk index | u32 LE |
| payload length | u32 LE |
| BLAKE3 hash of payload | 32 bytes |
| payload | length bytes |

The receiver rejects frames whose file index, chunk index or length don't match the
offer, and verifies the hash before writing. Duplicate chunks are ignored, so
retransmitting after a reconnect is always safe. The sender flushes the TLS stream
after every chunk.

## Receiver journal

Stored in the app's private `journals` folder while a transfer is incomplete, and
deleted once it finishes. The journal file is exclusively locked, so two receivers
can never write the same transfer at once.

| Record | Layout |
|---|---|
| chunk | `1`, file u32, chunk u32, hash 32 bytes |
| checkpoint | `2`, covered u64: number of chunk records now durable |
| file done | `3`, file u32, name length u16, final relative path |

- Every 2 seconds the receiver snapshots the chunk-record count, flushes the part
  files written since the last checkpoint, then writes a checkpoint record. Writers
  keep going during the flush.
- A file is flushed to disk before it is renamed to its final name, so a finished
  file is always complete.
- On resume, chunks covered by a checkpoint are trusted. Later chunks are
  re-hashed from the part file, and only matching ones count as received. A file
  whose done record was lost is recognized under its final name by its chunk
  hashes. A truncated final record from a crash is ignored.

## USB

Phone to phone, through Android Open Accessory (AOA):

1. The phone powering the cable (the USB host) asks for access to the other phone,
   checks that it speaks AOA (control request 51), sends Nearvolt's identity
   strings (52) and switches it into accessory mode (53).
2. The other phone reappears as Google's accessory device (`18D1:2D00` or
   `2D01`). Android opens Nearvolt on both phones for it.
3. Each phone copies between the cable's bulk endpoints and a local socket pair on
   two threads; the engine owns the other end of the pair.
4. The engine runs a yamux session over that socket: the host is the yamux client,
   the other phone the server. Each TLS stream is one yamux stream with the
   standard 256 KiB window.

Every USB write ends in a short packet (a write whose length is a multiple of 64
bytes is split into two), so the other side's read returns at once instead of
waiting to fill its buffer. Replugging the cable starts a new yamux session; the
sender's reconnect logic picks it up and resumes.
