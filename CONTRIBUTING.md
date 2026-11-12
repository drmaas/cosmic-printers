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

Maintainers cut a release like this:

1. On `main`, set the same version in `Cargo.toml` (`version`) and `arch/PKGBUILD` (`pkgver`). Use `X.Y.Z` with no `v` prefix in those files. Set `pkgrel` to `1` for a new version.
2. Merge that change to `main`.
3. Tag the release commit and push the tag:

```sh
git tag -a vX.Y.Z -m "vX.Y.Z"
git push origin vX.Y.Z
```

4. Pushing a `v*` tag starts `.github/workflows/release-arch.yml`. The job runs `scripts/check-version.sh` against the tag, builds `cosmic-printers-*.pkg.tar.zst`, and publishes a GitHub Release with that package attached. Release notes come from `gh release create --generate-notes`.
5. Confirm the Release page has the `.pkg.tar.zst` file. Installers follow the steps in `README.md`.

You can also run the Arch workflow by hand with `workflow_dispatch`. That builds and uploads the package as a workflow artifact. It only creates or updates a GitHub Release when the run is on a tag.

To rebuild the package from the working tree on Arch without waiting for CI, follow the comments at the top of `arch/PKGBUILD`.

## Tests

Unit tests cover status text, icon choice, `lpstat` parsing, and queue order. They must not need a running CUPS daemon or a live D-Bus bus. If a change needs a real printer, describe the manual check in the pull request instead of adding a test that only passes on your machine.
