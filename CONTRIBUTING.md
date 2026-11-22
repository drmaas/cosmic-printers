# Contributing

Thanks for looking at the printer applet. Issues and pull requests are welcome.

## Before you open a pull request

- Run `cargo fmt --all`.
- Run `cargo test --locked`.
- Run `cargo clippy --all-targets --locked -- -D warnings`, or `just check` after the lockfile is current.
- Keep the change focused. Do not mix a behavior fix with an unrelated refactor.
- Do not add a second printer administration UI. Queue, status, and launching the existing printer settings program are the scope of this applet.

## Versions

`Cargo.toml` `version` and `arch/PKGBUILD` `pkgver` must be the same. A release tag `vX.Y.Z` must match both. `scripts/check-version.sh` checks that on a tagged build.

## Releases

The Arch package is the only release artifact. Do not add deb, rpm, Nix, or Flatpak packaging unless that is an explicit project decision.

From a clean `main` that matches `origin/main`, run the release script. It bumps `Cargo.toml`, `arch/PKGBUILD` (`pkgver`, `pkgrel=1`), and `Cargo.lock`, runs `just check` and `just test`, commits, pushes `main`, pushes the `v*` tag, and watches the Arch release workflow:

```bash
scripts/release.sh minor          # or major, patch, or vX.Y.Z
# just release minor
```

Useful flags: `--dry-run` (print the plan only), `--yes` (no prompt), `--skip-check` (skip `just check` / `just test`).

Pushing a `v*` tag starts `.github/workflows/release-arch.yml`. The job runs `scripts/check-version.sh` against the tag, builds `cosmic-printers-*.pkg.tar.zst`, and publishes a GitHub Release with that package attached. Release notes come from `gh release create --generate-notes`. The same workflow also runs on pushes to `main` when Rust sources or the PKGBUILD change, to warm a shared Cargo cache that tagged release builds restore. CI on `main` already runs `cargo test --locked`, so the release job passes `--nocheck` to `makepkg`.

Confirm the Release page has the `.pkg.tar.zst` file. Installers follow the steps in `README.md`.

To verify versions without releasing:

```bash
scripts/check-version.sh vX.Y.Z
```

You can also run the Arch workflow by hand with `workflow_dispatch`. That builds and uploads the package as a workflow artifact. It only creates or updates a GitHub Release when the run is on a tag.

To rebuild the package from the working tree on Arch without waiting for CI, follow the comments at the top of `arch/PKGBUILD`.

## Tests

Unit tests cover status text, icon choice, `lpstat` parsing, and queue order. They must not need a running CUPS daemon or a live D-Bus bus. If a change needs a real printer, describe the manual check in the pull request instead of adding a test that only passes on your machine.
