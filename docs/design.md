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

1. **Home.** Two large cards, Send and Receive, and a "Check for updates" button
   that downloads and installs a new version in place. Nothing looks for other
   phones, or for updates, until asked.
2. **Send.** An optional "Choose files" step, then "How do you want to connect?"
   with four choices: Tap phones (only on phones with NFC), Scan code, Nearby
   phones and USB cable. Only the chosen one runs. Files chosen first are sent
   the moment the connection is set up, so choosing files and then tapping is
   enough; with none chosen, files are added after connecting.
3. **Tap phones, USB cable.** One picture and one instruction; the connection
   opens by itself when the phones touch or the cable answers.
4. **Nearby phones.** Phones with Receive open, as round avatars with names.
   Anything a method still needs (a permission, NFC, Wi-Fi, Bluetooth,
   Location) shows as one notice at the top with the button that fixes it;
   Receive shows the same. NFC off, on Tap phones and on Receive, is also asked
   about when the page opens ("Turn on NFC?"), since Android only lets the user
   switch it on, in its settings.
5. **Receive.** A large QR code, the name others see, and the ways to connect.
6. **Receive prompt.** One sheet, "Rahul's Pixel 9 wants to connect and send you
   files", with the 4-digit code when there is one, and Accept and Decline.
   Asked once per connection, and never after a tap.
7. **Connection.** Both phones: who they're connected to and how, the batch on
   the move as a large progress ring with percent, speed, time left and file
   count, and every batch so far, newest first. While the receiver decides it
   shows the 4-digit code to compare. "Connected" appears only once the other
   phone has accepted. Until then, and while reconnecting, the screen offers
   Cancel (or Stop reconnecting) and no Add files. Once connected, the sender's
   "Add files" asks Files or Folder and queues them; "Disconnect" on either
   phone ends it for both, after a confirmation. Back does the same.
