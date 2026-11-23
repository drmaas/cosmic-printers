#!/bin/sh
# Install cosmic-printers from a GitHub Release Arch package.
#
# Usage:
#   scripts/install-applet.sh [--dry-run] [--yes] [latest|X.Y.Z|vX.Y.Z]
#
# Downloads the release .pkg.tar.zst and installs it with pacman -U.
# This is not `just install` (local build into /usr).
#
# The package is unsigned; trust is HTTPS to GitHub Releases, same as a
# manual download from the Releases page. Run this script from a clone or a
# saved copy — do not pipe an unpinned script into sh.
set -eu

dry_run=0
assume_yes=0
version=

repo=drmaas/cosmic-printers
api_base=https://api.github.com/repos/$repo
user_agent=cosmic-printers-install-applet
asset_glob='cosmic-printers-*-x86_64.pkg.tar.zst'

usage() {
    echo "usage: scripts/install-applet.sh [--dry-run] [--yes] [latest|X.Y.Z|vX.Y.Z]" >&2
}

die() {
    echo "$*" >&2
    exit 1
}

usage_die() {
    usage
    echo "$*" >&2
    exit 2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --dry-run) dry_run=1 ;;
        --yes|-y) assume_yes=1 ;;
        -h|--help)
            usage
            exit 0
            ;;
        -*)
            usage_die "unknown option: $1"
            ;;
        *)
            if [ -n "$version" ]; then
                usage_die "unexpected argument: $1"
            fi
            version=$1
            ;;
    esac
    shift
done

if [ -z "$version" ]; then
    version=latest
fi

case "$version" in
    latest) ;;
    v[0-9]*.[0-9]*.[0-9]*|[0-9]*.[0-9]*.[0-9]*)
        version=${version#v}
        case "$version" in
            *.*.*.*) usage_die "version must be X.Y.Z: $version" ;;
            [0-9]*.[0-9]*.[0-9]*) ;;
            *) usage_die "version must be X.Y.Z or latest" ;;
        esac
        # Reject non-numeric segments quietly via pattern already; still block junk.
        case "$version" in
            *[!0-9.]*|.*|*..*|*. ) usage_die "version must be X.Y.Z or latest" ;;
        esac
        ;;
    *)
        usage_die "expected latest, X.Y.Z, or vX.Y.Z; got: $version"
        ;;
esac

command -v curl >/dev/null 2>&1 || die "curl not found"
command -v pacman >/dev/null 2>&1 || die "pacman not found (Arch package install only)"
command -v mktemp >/dev/null 2>&1 || die "mktemp not found"

machine=$(uname -m)
[ "$machine" = "x86_64" ] || die "unsupported architecture: $machine (need x86_64)"

# curl -q must be first so ~/.curlrc cannot disable TLS checks.
curl_q() {
    curl -q --proto-redir =https "$@"
}

fetch_release_json() {
    # fetch_release_json <url>
    curl_q -fsSL \
        -H "Accept: application/vnd.github+json" \
        -H "User-Agent: $user_agent" \
        -- "$1"
}

# Extract every Arch package download URL. Works on single-line GitHub JSON.
asset_urls_from_json() {
    # Reads JSON on stdin; prints matching URLs, one per line.
    grep -oE 'https://github\.com/[^"]+/cosmic-printers-[^"/]+-x86_64\.pkg\.tar\.zst' || true
}

tag_from_json() {
    # Prefer the first tag_name (release object), not later nested copies.
    grep -oE '"tag_name"[[:space:]]*:[[:space:]]*"[^"]+"' \
        | head -n 1 \
        | sed -n 's/.*"\([^"]*\)"[[:space:]]*$/\1/p'
}

if [ "$version" = "latest" ]; then
    release_url="$api_base/releases/latest"
else
    release_url="$api_base/releases/tags/v${version}"
fi

json=$(fetch_release_json "$release_url") || die "failed to fetch release metadata from GitHub"

tag=$(printf '%s\n' "$json" | tag_from_json)
[ -n "$tag" ] || die "could not read tag_name from release metadata"

urls=$(printf '%s\n' "$json" | asset_urls_from_json)
count=$(printf '%s\n' "$urls" | sed '/^$/d' | wc -l | tr -d ' ')

if [ "$count" -eq 0 ]; then
    die "no Arch package asset matching $asset_glob on $tag"
fi
if [ "$count" -ne 1 ]; then
    die "expected exactly one Arch package asset on $tag, found $count"
fi

asset_url=$(printf '%s\n' "$urls" | sed '/^$/d' | head -n 1)
case "$asset_url" in
    https://github.com/*) ;;
    *) die "refusing download host (want https://github.com/...): $asset_url" ;;
esac

asset_name=${asset_url##*/}

if [ "$dry_run" -eq 1 ]; then
    echo "dry run only; no changes made"
    echo "  tag:    $tag"
    echo "  asset:  $asset_name"
    echo "  url:    $asset_url"
    if [ "$assume_yes" -eq 1 ]; then
        echo "  install: sudo pacman -U --noconfirm <downloaded-package>"
    else
        echo "  install: sudo pacman -U <downloaded-package>"
    fi
    exit 0
fi

if [ "$assume_yes" -ne 1 ]; then
    if [ -z "${INSTALL_APPLET_FORCE_TTY:-}" ] && [ ! -t 0 ]; then
        die "stdin is not a terminal; re-run with a TTY or pass --yes"
    fi
fi

if [ "$(id -u)" -eq 0 ]; then
    sudo_cmd=
else
    command -v sudo >/dev/null 2>&1 || die "sudo not found (needed to install as root)"
    sudo_cmd=sudo
fi

tmpdir=$(mktemp -d)
cleanup() {
    rm -rf -- "$tmpdir"
}
trap cleanup EXIT

pkg_path="$tmpdir/$asset_name"

curl_q -fsSL \
    -H "User-Agent: $user_agent" \
    -o "$pkg_path" \
    -- "$asset_url" || die "failed to download $asset_name"

[ -s "$pkg_path" ] || die "downloaded package is empty: $asset_name"

if [ "$assume_yes" -eq 1 ]; then
    pacman_args="-U --noconfirm"
else
    pacman_args="-U"
fi

# Intentional word-splitting for pacman flags.
# shellcheck disable=SC2086
if [ -n "$sudo_cmd" ]; then
    $sudo_cmd pacman $pacman_args -- "$pkg_path"
else
    pacman $pacman_args -- "$pkg_path"
fi

echo "Installed $asset_name ($tag)"
echo "Add Printers to the COSMIC panel from Settings → Desktop → Panel if needed."
