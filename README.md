# Remind Me

Remind Me is a small, local-only reminder app for the Linux desktop. It uses
Rust, GTK4, and libadwaita to provide quick one-time reminders with actionable
desktop notifications.

## Features

- Quick reminders with a message, date, and time
- Done and ten-minute snooze actions in the app and notifications
- Upcoming reminders grouped as Overdue, Today, Tomorrow, and Later
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
