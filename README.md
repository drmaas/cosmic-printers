# Printers for COSMIC

Printers is a panel applet for the COSMIC desktop. It shows the print queue, printer warnings, and shortcuts into printer settings.

This is an unofficial community project. It is not made or endorsed by System76.

The applet follows the same jobs you can manage from the Cinnamon printers applet: which printers exist, which ones need attention, and the jobs waiting on each one. It talks to CUPS. It does not replace the printer setup program.

## Install

Download the latest `cosmic-printers-*.pkg.tar.zst` from the [Releases](https://github.com/drmaas/cosmic-printers/releases) page and install it with pacman:

```sh
sudo pacman -U ./cosmic-printers-*.pkg.tar.zst
```

Then add it to the panel:

1. Open Settings.
2. Go to Desktop, then Panel.
3. Add **Printers**.

`cups` is required. `system-config-printer` is optional. Without it, the applet can still show the queue, but the Printers button and per-printer settings shortcut stay disabled. Printer properties open only when `system-config-printer` has registered `org.fedoraproject.Config.Printing` on the session bus.

The panel button can hide when you have no printers, or only while jobs exist. COSMIC still keeps the applet's panel slot when the button draws nothing, so the gap can remain until you remove the applet. Use **Always** if you want the button to stay visible.

## Build from source

Install Rust (stable, 1.93 or newer), `just`, and the libraries COSMIC applets link: wayland, xkbcommon, fontconfig, freetype, expat, dbus, and a Vulkan loader. On Arch:

```sh
sudo pacman -S --needed rust just wayland libxkbcommon fontconfig freetype2 expat dbus vulkan-icd-loader cups
```

`cups` supplies `lpstat`, `lp`, and `cancel`, which the applet runs at runtime.

```sh
just build-release
just run
sudo just install
```

`just check` runs clippy. `just test` runs the unit tests. Those tests do not need a printer or a D-Bus session.

## License

[GPL-3.0-only](LICENSE).
