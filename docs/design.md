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

1. **Home.** Nearby devices as round avatars with names. A large "Choose files"
   button. On laptops, files can be dropped onto a device avatar.
2. **Pick.** Files, folders, photos and videos with a running total size. One
   "Send" button.
3. **Receive prompt.** One card, for example "Rahul's Pixel wants to send 248 files
   (312 GB)", with Accept and Decline.
4. **Transfer.** A large progress ring with percent, speed, time left and file
   count. It ends with "All 248 files verified".
