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

## Tests

Unit tests cover status text, icon choice, `lpstat` parsing, and queue order. They must not need a running CUPS daemon or a live D-Bus bus. If a change needs a real printer, describe the manual check in the pull request instead of adding a test that only passes on your machine.
