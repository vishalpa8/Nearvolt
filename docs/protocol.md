# Nearvolt wire protocol, version 5

## Transport

- TLS 1.3 only. ALPN: `nearvolt/1`.
- Over Wi-Fi, every stream is a TCP connection. Over a USB cable, every stream is
  a yamux stream multiplexed on the one cable (see [USB](#usb)). TLS and
  everything above it are identical on both.
- Both sides present a self-signed Ed25519 certificate (the device identity).
  No certificate authorities are used.
- With a **private ticket** (QR code or NFC tap) the sender pins the receiver's
  certificate: the BLAKE3 hash of its DER encoding must equal the fingerprint in
  the ticket. Without one (USB), both devices confirm a 4-digit
  code instead.
- A connection uses one **control stream** and several **data streams** (8 by
  default). Parallel streams spread encryption over CPU cores and let the
  kernel's TCP offloads carry the bulk of the work.

Why TCP rather than QUIC: on the development PC, the best QUIC configuration
(quinn, several connections and sockets) reached about 300 MB/s over loopback,
while 8 TCP connections reached 919 MB/s. Kernel TCP also uses less CPU per byte,
which matters for phone battery and heat.

## Ticket

The receiver creates tickets for each receive session. Text form: `NV1`
followed by unpadded RFC 4648 base32, so it fits the QR alphanumeric mode.

- The **QR ticket** carries a secret that is new for each receive session. It's only shown on screen.
- The **tap ticket** carries a different secret, also new for each receive session. It's only served over
  NFC, which works at a few centimetres. Holding the phones together is the
  consent, so a sender with this secret connects without a prompt.
- The **public ticket** is the same without any secret. Nothing advertises it
  any more (phones are no longer listed over the network or Bluetooth), so only
  tests use it.

Binary layout, all integers big-endian:

| Field | Size |
|---|---|
| version (1) | 1 byte |
| flags: 1 = has secret, 2 = has Wi-Fi Direct group | 1 byte |
| receiver certificate fingerprint (BLAKE3) | 32 bytes |
| per-session secret, if flag 1 | 16 bytes |
| address count (at most 16) | 1 byte |
| each address: family (4 or 6), IP bytes, TCP port | 7 or 19 bytes |
| device name | 1-byte length, then UTF-8 (at most 64 bytes) |
| if flag 2: group SSID, passphrase | each 1-byte length, then UTF-8 |
| if flag 2: group frequency in MHz | 2 bytes |

The tickets are refreshed when the receiver's Wi-Fi Direct group comes up, so
they also carry the group's address. A sender that finds a group in the ticket
joins it (no pop-up on Android 10 and later) and connects to the receiver's
address in the group alone. If joining fails, it tries every address in the
ticket at once. Once one address has worked, every later stream and every
reconnect uses only that address.

## Connection

A connection lasts until either phone sends `bye`. While it's open the sender
can offer any number of batches; they go one after another, numbered from 0.
The receiver approves the connection once, not each batch.

Each control message is a 4-byte little-endian length followed by JSON (at most
64 MiB).

```
sender                                               receiver
  | -- hello {version, secret | commitment, name} ---> |   secret compared in constant time
  | <-- welcome {name, session, nonce?} | reject ----- |   session: random 16-byte ID
  | -- reveal {nonce} (code pairing only) -----------> |   must match the commitment
  |                                                    |   both screens show the code
  | <-- ready | reject ------------------------------- |   asks the user, unless tapped or
  |                                                    |   reconnecting from the same sender
  | -- offer {batch, files[]} -----------------------> |   paths cleaned, space checked
  | <-- accept {batch, have[]} | reject -------------- |   have[] = none | complete | partial {ranges}
  |                                                    |
  | == data streams: hello {secret?, session}, then files ==> |
  |                                                    |
  | -- ping (every 500 ms) --------------------------> |
  | <-- progress {files, bytes, checked} (500 ms) ---- |   progress of the batch, doubles as heartbeat
  | <-- redo {file, offset, len} (rare) -------------- |   a chunk failed its check; send it again
  | <-- complete {batch, files, bytes} --------------- |
  | -- offer {batch: 1, ...} ------------------------> |   the next batch, if any
  |                       ...                          |
  | -- bye ------------------------------------------> |   either side; the other stops too
```

- `offer.files[i]` is `{path, size, modified}`. `path` is relative and uses `/`
  separators. `modified` is milliseconds since the Unix epoch, when known.
- A data stream is accepted only if its session matches the current control
  stream and its certificate matches the sender's.
- While a phone is connected, the receiver turns away every other sender. While the\n  user is deciding about one phone, a different one is turned away too (eject\n  with kind usy); the same phone asking again, or a tap, takes over. Every\n  eject carries a kind (declined, usy, old_version, disk_full, ...), so\n  the app can say what happened in plain words instead of showing the reason text.\n- Connections that fail the handshake are ignored and counted, never fatal: at\n  most 32 handshakes run at once and the rest are dropped, so a flood of junk\n  connections can't end Receive or a transfer in progress.
- Eight seconds without a message on the control stream means the link is
  gone; shorter pauses, such as a phone's Wi-Fi briefly stalling, are left to
  TCP. The sender reconnects through the same address, with backoff from 100 ms
  up to 5 s, until it gets through, the user disconnects, or 5 minutes pass; the
  receiver likewise stops waiting after 5 minutes, and both phones then show
  the connection as ended with a reason, so a phone that left for good doesn't
  keep the other one retrying. The backoff starts
  over only after a connection that lasted 5 seconds, so a receiver that hangs
  up at once is retried more and more slowly. The receiver lets the same sender
  back in without asking, and the sender re-offers the batch it was on.
- `bye` is a deliberate end, so the receiver throws away the batch in progress:
  the half-received file, its journal and any folder made only for it.
  Complete, verified files stay. A dropped link, or a phone that vanishes
  without saying `bye`, is not deliberate: the half-received file is kept so
  the transfer can resume.
- A batch the receiver can't go on with after accepting it, because of full storage,\n  a file it can't save, or a file that keeps arriving damaged (3 tries), is ended\n  with `abort {batch, kind}`: the receiver throws away the batch, both phones report\n  it as failed with the kind, and the connection stays. Free space is checked when a\n  batch opens (counting only what's still missing) and again before each file.\n- A batch the receiver can't take, for example for lack of space, is rejected
  on its own; the connection stays open for the next one.
- A batch the sender can't finish, because a file can't be read any more, is
  ended with `cancel {batch}`: the receiver throws away what it has of that
  batch (files already complete and verified stay), both phones report the batch
  as failed, and the connection carries on with the next one.
- When Receive opens, part files and journals not touched for 7 days are
  deleted: a transfer nobody came back for in a week can't be resumed, and they
  would otherwise stay for good. Only files Nearvolt itself named are touched.
- If nothing moves for 30 seconds while a batch is open, the sender restarts the
  connection so the receiver can report what's missing.

The receiver derives each batch's transfer ID as the BLAKE3 hash of the
sender's certificate fingerprint and the canonical offer. Re-offering the same
files after a reconnect maps to the same ID, which is how resume finds the
journal.

## Code pairing

Used when the sender has no secret: the USB cable.

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

## Files on a data stream

A file is cut into chunks of up to 8 MiB. The sender hands the chunks of a batch to
its streams in offer order, each stream taking the next chunk as soon as it's free,
so one big file travels over every stream at once, and small files still move
back to back. The chunks of a file may arrive in any order.

| Field | Size |
|---|---|
| batch | u32 LE |
| file index | u32 LE |
| offset | u64 LE |
| length | u64 LE |
| bytes `offset .. offset + length` of the file | length bytes |
| BLAKE3 hash of those bytes | 32 bytes |

- A chunk is at most 8 MiB, and an empty file is one chunk of length 0. After a
  reconnect the receiver reports the ranges it has (`partial {ranges}`) and the
  sender sends only the gaps.
- The receiver rejects a header whose batch or file isn't in the offer, or whose
  range runs past the end of the file or is longer than 64 MiB.
- Each chunk is hashed as it arrives and compared with the sender's hash. Only a
  match counts the range as received. A mismatch sends `redo` for that chunk
  alone; a chunk that keeps arriving damaged (3 times) fails the batch.
- When every range of a file is in, the receiver flushes the part file and renames
  it to its final name. A chunk cut off by a broken stream counts for nothing.

## Receiver journal

Stored in the app's private `journals` folder while a batch is incomplete, and
deleted once it finishes. The journal file is exclusively locked, so two
receivers can never write the same batch at once.

| Record | Layout |
|---|---|
| on disk | `1`, file u32, start u64, end u64: these bytes of the file are flushed and checked |
| file done | `3`, file u32, name length u16, final relative path |

- Every 2 seconds the receiver flushes the part files written since the last
  checkpoint, then appends an "on disk" record for each range finished since.
  Writers keep going during the flush; the ranges were taken before it, so it
  never claims bytes that weren't flushed.
- On resume, a file restarts from its last "on disk" length, so at most the
  last 2 seconds are sent again. A file whose done record survived but whose
  rename didn't is renamed then. A truncated final record from a crash is
  ignored.

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
