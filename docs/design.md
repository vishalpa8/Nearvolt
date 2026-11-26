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

1. **Home.** Two large cards, Send and Receive, plus an update banner when there
   is one. Nothing looks for other phones until one is chosen.
2. **Send.** Nearby devices with Receive open, as round avatars with names, plus
   "Scan code" and a tap hint. One "Choose files" button asks Files or Folder.
   Anything Android still needs (a permission, Wi-Fi, Bluetooth, Location) shows
   as one notice at the top with the button that fixes it; Receive shows the same.
3. **Ready to send.** Total size and file count, the devices to send to, and the
   picked files and folders. One "Send to …" button.
4. **Receive.** A large QR code, the name others see, and the ways to connect.
5. **Receive prompt.** One sheet, for example "Rahul's Pixel 9 wants to send you
   4 files · 821 MB", with the 4-digit code when there is one, and Accept and
   Decline.
6. **Transfer.** A large progress ring with percent, speed, time left and file
   count. While waiting for the receiver it shows the 4-digit code to compare. It
   ends with "All 4 files verified".
