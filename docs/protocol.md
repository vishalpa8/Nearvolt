# Proxima wire protocol, version 1

## Transport

- TLS 1.3 only, over TCP. ALPN: `proxima/1`.
- Both sides present a self-signed Ed25519 certificate (the device identity).
- The sender pins the receiver's certificate: the BLAKE3 hash of its DER encoding
  must equal the fingerprint in the ticket. No certificate authorities are used.
- A session uses one **control connection** and several **data connections**
  (8 by default). Parallel connections spread encryption over CPU cores and let
  the kernel's TCP offloads carry the bulk of the work.

Why TCP rather than QUIC: on the development PC, the best QUIC configuration
(quinn, several connections and sockets) reached about 300 MB/s over loopback,
while 8 TCP connections reached 919 MB/s. Kernel TCP also uses less CPU per byte,
which matters for phone battery and heat.

## Ticket

The receiver creates a ticket for each receive session and shares it by QR code,
NFC or Bluetooth. Text form: `PX1` followed by unpadded RFC 4648 base32, so it fits
the QR alphanumeric mode.

Binary layout, all integers big-endian:

| Field | Size |
|---|---|
| version (1) | 1 byte |
| receiver certificate fingerprint (BLAKE3) | 32 bytes |
| one-time secret | 16 bytes |
| address count | 1 byte |
| each address: family (4 or 6), IP bytes, TCP port | 7 or 19 bytes |
| device name length | 1 byte |
| device name (UTF-8, at most 64 bytes) | variable |

## Control messages

Each message is a 4-byte little-endian length followed by JSON (at most 64 MiB).

```
sender                                     receiver
  | -- hello {version, secret, name} ------> |   secret compared in constant time
  | <-- welcome {name, session} | reject ----|   session: random 16-byte ID
  | -- offer {chunk_size, files[]} --------> |   paths cleaned, space checked, user approves
  | <-- accept {have[]} | reject ------------|   have[] = none | complete | partial bitmap
  |                                          |
  | == data connections: hello {secret, session}, then chunk frames ==> |
  |                                          |
  | -- ping (every 500 ms) ----------------> |
  | <-- progress {files, bytes} (500 ms) ----|   verified progress, doubles as heartbeat
  | <-- resend {chunks[]} (rare) ------------|   on-disk verification failed
  | <-- complete {files, bytes} -------------|
```

- `offer.files[i]` is `{path, size, modified}`. `path` is relative and uses `/`
  separators. `modified` is milliseconds since the Unix epoch.
- A data connection is accepted only if its secret is valid, its session matches
  the current control connection, and its certificate matches the sender's.
- Four seconds without a message on the control connection means the link is
  gone. Both sides drop the session; the sender reconnects and re-offers.
- If the sender has nothing left to send and verified progress stops for 30
  seconds, it restarts the session so the receiver can report what's missing.

The receiver derives the transfer ID as the BLAKE3 hash of the sender's
certificate fingerprint and the canonical offer. Re-offering the same files after a
reconnect maps to the same ID, which is how resume finds the journal.

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

Stored in `<destination>/.proxima/<transfer id>/journal` while a transfer is
incomplete, and deleted once it finishes. The journal file is exclusively locked,
so two receivers can never write the same transfer at once.

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
