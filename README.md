# tegami

Mail for GNOME, in Rust: a libadwaita shell sitting on a UI-free core.

Version 0.1.0, so treat it as alpha.

## Building

Meson drives the build, and bare cargo works too; `crates/app/build.rs` compiles
the Blueprint UI itself when Meson isn't there to do it.

```sh
meson setup build
meson compile -C build
meson test -C build
meson install -C build
```

`meson install` goes to `/usr/local` unless you pass `--prefix`, so expect to need
root or a `--destdir` escape hatch.

You need meson 1.4 or newer, `blueprint-compiler`, `glib-compile-schemas` and
`glib-compile-resources`, and the GTK4, libadwaita and WebKitGTK 6.0 development
packages. The Flatpak manifest at `build-aux/flatpak/it.dottorblaster.tegami.json`
targets `org.gnome.Platform` 50 and is the least fiddly way in.

## Tests

Bring up the GreenMail IMAP/SMTP server first:

```sh
docker compose up -d
TEGAMI_GREENMAIL_HOST=localhost dbus-run-session -- cargo test --workspace
```

`dbus-run-session` is not optional. A few tests want a session bus and will wedge
without one, which is why CI wraps the same command.

Clippy, exactly as CI runs it:

```sh
cargo clippy --workspace --all-targets -- -D warnings
```

MSRV is 1.93. relm4 and gtk4-rs set that floor, not us, so don't bump it casually
and don't count on it dropping.

## Layout

`crates/mail-core` holds the mail model, the IMAP and SMTP backends, the SQLite
store, sync and the outbox. No GTK in there by design. `crates/accounts` wraps
GNOME Online Accounts for discovery and credentials. `crates/app` is the shell.

State lives in `~/.local/share/tegami`: `mail.sqlite3`, cached bodies and queued
outbound MIME, with extracted attachments under the cache dir instead. Delete the
directory if you want a clean slate.

## License

GPL-3.0-or-later.
