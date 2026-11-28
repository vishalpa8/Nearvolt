# Nearvolt design

Clean, light and calm. Anyone should be able to send 1 TB without reading
instructions.

## Principles

1. **One obvious action per screen.**
2. **Nothing that isn't needed.** No ads, feeds, games, sign-up or settings maze.
3. **Show the truth.** Real speed, real time left, and proof of verification at the
   end.
4. **Problems fix themselves quietly.** A dropped link shows "Reconnecting…" for a
   moment, then continues.

## Color

| Token | Value | Use |
|---|---|---|
| background | `#F7F7F5` | Page background, warm off-white |
| surface | `#FFFFFF` | Cards and sheets |
| surfaceMuted | `#EFEFEC` | Pressed states, progress track |
| border | `#E4E4E0` | Hairline dividers |
| textPrimary | `#17181A` | Headings and body |
| textSecondary | `#6B6E73` | Supporting text |
| accent | `#2457F5` | The one action color: buttons, progress, selection |
| accentSoft | `#E8EEFE` | Accent backgrounds |
| success | `#1F9D55` | Verified and complete |
| warning | `#C27C0E` | Reconnecting, low battery |
| danger | `#D93636` | Errors and decline |

Light theme only at launch.

Text on a status colour must reach 4.5:1 (the minimum for small text). The success
green and the warning amber are too light to carry words on their own tinted fill, so
pills and labels darken them with `readableOn` in `lib/theme.dart`; the dot, icon and
ring keep the brand colour. `test/contrast_test.dart` checks this.

Every screen must also fit a small phone, landscape and a tablet at 100%, 150% and 200%
text, with nothing cut off or overflowing: `test/layout_test.dart`.

## Files and what happened to them

A batch row says what became of it: **Waiting its turn**, **Sending** / **Receiving**,
**Sent and verified** / **Received and verified**, **Couldn't be sent** with the
reason, or **Cancelled · not sent** / **Cancelled · not received** when the connection
ended first. When a connection ends, anything still waiting or moving becomes
cancelled, so no row keeps saying "Sending" for a transfer that is over. The line under
an ended connection says what was and was not finished.

Files picked on the Send screen are used up as soon as a connection takes them, whether
they are sent, cancelled or fail: the screen goes back to "Choose files", and they are
never offered to the next connection.

## Type

Inter, with system fallbacks.

| Style | Size / line height | Weight |
|---|---|---|
| Display | 34 / 40 | 600 |
| Title | 22 / 28 | 600 |
| Body | 16 / 24 | 400 |
| Label | 14 / 20 | 500 |
| Caption | 12 / 16 | 400 |

Numbers in progress views use tabular figures so they don't jitter.

## Shape and space

- Spacing scale: 4, 8, 12, 16, 24, 32, 48.
- Corner radius: 12 for buttons, 20 for cards, 28 for sheets.
- Elevation: one soft shadow for floating sheets only; everything else is flat with
  hairline borders.
- Touch targets at least 48 × 48.
- Motion: 200–300 ms ease-out, and reduced motion is respected.

## Screens

Golden screenshots of each one are in [app/test/goldens](../app/test/goldens).

1. **Home.** Two large cards, Send and Receive, a "Share Nearvolt" link that opens
   the phone's share sheet with where to get the app, and a "Check for updates"
   button. A new version, found by that button or by the quiet check a few seconds
   after opening (and every 6 hours), is offered first and installed only on Update.
   Nothing looks for other phones until a way to connect is chosen.
2. **Send.** An optional "Choose files" step, then "How do you want to connect?"
   with three choices: Tap phones (only on phones with NFC), Scan code and
   USB cable. Only the chosen one runs. Files chosen first are sent
   the moment the connection is set up, so choosing files and then tapping is
   enough; with none chosen, files are added after connecting.
3. **Tap phones, USB cable.** One picture and one instruction; the connection
   opens by itself when the phones touch or the cable answers.
4. **What a method needs.** Anything it uses (a permission, NFC, Wi-Fi, Location)
   must be on first: until it is, the page doesn't open. A "Turn these on to
   continue" screen lists what's off, each with its own button, and nothing
   (the network, NFC) is started behind it. The page opens by itself
   when the last one is on, and closes again if one is turned off later. Receive
   works the same, and Scan code asks before the camera opens.
5. **Receive.** A large QR code, the name others see, and the ways to connect.
6. **Receive prompt.** One sheet, "Rahul's Pixel 9 wants to connect and send you
   files", with the 4-digit code when there is one, and Accept and Decline.
   Asked once per connection, and never after a scan or a tap.
7. **Connection.** Both phones: who they're connected to and how, the batch on
   the move as a large progress ring with percent, speed, time left and file
   count, and every batch so far, newest first. Each batch lists its files by name:
   the one moving shows a bar and how much has moved, finished ones a tick, waiting
   ones their size. A big batch shows the files around the one moving, with how many
   are done above and waiting below. Every waiting or moving file has a cross to skip
   just that file, and every batch that hasn't finished has Cancel batch; both ask first,
   and nothing else is affected. Either phone can do either. The notification says the
   same thing on both phones (Sending to / Receiving from, files and bytes, and at the end
   Sent or Received N files). While the receiver decides it
   shows the 4-digit code to compare (after a scan it says it is waiting for the other
   phone to accept). The sender has no code to confirm: comparing it on the receiver,
   when accepting, is the one check. "Connected" appears only once the other
   phone has accepted. Until then, and while reconnecting, the screen offers
   Cancel (or Stop reconnecting) and no Add files. Once connected, the sender's
   "Add files" asks Files or Folder and queues them; "Disconnect" on either
   phone ends it for both, after a confirmation. Back does the same.
