# Agents

This file is the source of truth for coding agents. `CLAUDE.md` points here.

## What this is

Unofficial COSMIC panel applet. Binary `cosmic-printers`. App id `io.github.drmaas.CosmicPrinters`. License GPL-3.0-only.

It lists printers and jobs, shows IPP warnings, cancels jobs, moves a queued job to the front, and launches `system-config-printer`. It is not a CUPS administration app. Do not add driver setup, printer sharing controls, or a second settings window.

## Commands

```sh
cargo fmt --all
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --release --locked
just run
```

`just install` copies the binary, desktop file, metainfo, and icon under `/usr`.

## Layout

- `src/main.rs` loads Fluent strings and starts `cosmic::applet::run`.
- `src/app.rs` is the panel button, popup, and confirm dialog.
- `src/cups.rs` owns the queue, `lpstat` parsing, process actions, and the `org.cups.cupsd.Notifier` subscription.
- `src/status.rs` is pure: reason mapping, icons, tooltips, visibility, send-to-front index. Keep shell and D-Bus calls out of this file.
- `src/config.rs` stores `ShowIcon` with `cosmic-config`.
- `i18n/en/cosmic_printers.ftl` is the English catalog. The `fl!` macro only accepts string literals, so new copy needs a match arm in `app.rs` as well as an `.ftl` line.

`libcosmic` comes from `https://github.com/pop-os/libcosmic.git`. Commit `Cargo.lock` when dependencies change.

## CUPS

Force `LC_ALL=C` for `lpstat`. Live updates come from the system bus interface `org.cups.cupsd.Notifier` at `/org/cups/cupsd/Notifier`. Cancel with `cancel <printer>-<id>`. Send to front with `lp -i <printer>-<id> -q 100`, then reorder the local `IndexMap`. The next notifier signal is the source of truth after that.

Tests must not start CUPS or connect to a bus. Parse sample `lpstat` text and apply `CupsSignal` values instead.

## Versions and packaging

`Cargo.toml` `version` and `arch/PKGBUILD` `pkgver` stay equal. Tag builds run `scripts/check-version.sh`. The Arch package is the only release artifact. Do not add deb, rpm, Nix, or Flatpak packaging unless asked.

User-facing docs (`README.md`, release notes) stay in plain language. This file can name modules and commands.
