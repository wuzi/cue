# Remind Me

Remind Me is a small, local-only scratchpad for the Linux desktop. Write notes
directly on its canvas, and turn any note into a one-time reminder by adding an
English `@schedule`.

## Features

- A seamless, persistent note canvas with no separate input form
- Inline reminders such as `Call Ada @tomorrow 9am`
- Offline English schedule parsing for relative times, weekdays, and dates
- Clickable, normalized schedule suffixes and an exact date/time picker
- Done and ten-minute snooze actions in the app and notifications
- Secondary Active Reminders and History pages
- Local SQLite persistence and completed reminder history
- Adaptive GNOME interface with system light, dark, and accent styles
- No accounts, analytics, network access, or broad filesystem permissions

## Native development build

Install Rust, Meson, Blueprint Compiler, GTK4 and libadwaita development
packages, then run:

```sh
meson setup build
meson compile -C build
./build/remind-me
```

On Fedora, the required packages are `rust`, `cargo`, `meson`,
`blueprint-compiler`, `gtk4-devel`, `libadwaita-devel`, `gettext-devel`, and
`sqlite-devel`.

## Flatpak build

Install Flatpak Builder and the GNOME 50 SDK, then run:

```sh
flatpak-builder --user --install --force-clean flatpak-build io.github.wuzi.RemindMe.json
flatpak run io.github.wuzi.RemindMe
```

Closing the window keeps the process alive while reminders are pending in the
current login session. Delivery after logout, reboot, or forced termination is
outside the first release; overdue reminders are delivered when the app next
starts.

## Reminder syntax

Write a note, then add an optional schedule after `@` to make it a reminder:

```text
Take a break @in 30 minutes
Call Ada @tomorrow at 9am
Submit the report @next Friday 14:30
```

Without a schedule, the entry remains a local note. The schedule grammar is
English-only in this release and is parsed entirely on your device. Use `@@`
when you need a literal boundary-style `@` in the note text.

Supported schedule forms are:

- Relative: `in 15 minutes`, `in an hour`, `in 2 days`, `in 1 week`
- Named days: `today`, `tomorrow`, weekdays, and `next Friday`
- Times: `9am`, `9:30 PM`, `14:30`, `noon`, and `midnight`
- Day parts: `morning`, `afternoon`, `evening`, and `tonight`
- Dates: `Aug 20`, `August 20 2026`, and `2026-08-20`

Named dates without a time use 09:00, except `today`, which uses the rounded
one-hour default. A bare weekday chooses its next future occurrence; `next`
chooses that weekday in the following calendar week. Time-only schedules and
month/day dates roll forward to their next future occurrence. Explicit past
dates and local times skipped by a daylight-saving clock change are rejected.
