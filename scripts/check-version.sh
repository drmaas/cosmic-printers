#!/bin/sh
# Fail when Cargo.toml, the Arch package version, and the git tag disagree.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: scripts/check-version.sh vX.Y.Z" >&2
    exit 2
fi

tag=${1#v}
cargo_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
pkgver=$(sed -n 's/^pkgver=//p' arch/PKGBUILD | head -n 1)

if [ "$cargo_version" != "$tag" ] || [ "$pkgver" != "$tag" ]; then
    echo "Version mismatch: tag ${tag}, Cargo.toml ${cargo_version}, pkgver ${pkgver}" >&2
    exit 1
fi
