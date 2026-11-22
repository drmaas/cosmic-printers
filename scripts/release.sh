#!/bin/sh
# Cut a tagged release: bump versions, sync Cargo.lock, commit, tag, push.
# Pushing the v* tag starts .github/workflows/release-arch.yml.
set -eu

usage() {
    echo "usage: scripts/release.sh [--dry-run] vX.Y.Z" >&2
    exit 2
}

dry_run=0
version=""

for arg in "$@"; do
    case "$arg" in
        --dry-run)
            dry_run=1
            ;;
        -h | --help)
            usage
            ;;
        -*)
            echo "unknown option: $arg" >&2
            usage
            ;;
        *)
            if [ -n "$version" ]; then
                usage
            fi
            version=$arg
            ;;
    esac
done

if [ -z "$version" ]; then
    usage
fi

version=${version#v}
case "$version" in
    [0-9]*.[0-9]*.[0-9]*)
        ;;
    *)
        echo "version must look like X.Y.Z (got: $version)" >&2
        exit 2
        ;;
esac

tag="v$version"
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "not a git repository" >&2
    exit 1
fi

branch=$(git rev-parse --abbrev-ref HEAD)
if [ "$branch" != "main" ]; then
    echo "must run on main (current branch: $branch)" >&2
    exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
    echo "working tree not clean; commit or stash first" >&2
    exit 1
fi

git fetch origin main
if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then
    echo "main is not up to date with origin/main; pull first" >&2
    exit 1
fi

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "tag $tag already exists locally" >&2
    exit 1
fi

if git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
    echo "tag $tag already exists on origin" >&2
    exit 1
fi

current=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
if [ "$current" = "$version" ]; then
    echo "Cargo.toml is already at $version" >&2
    exit 1
fi

echo "releasing $current -> $version"

edited=0
restore() {
    git checkout -- Cargo.toml arch/PKGBUILD Cargo.lock
}
trap 'if [ "$dry_run" -eq 1 ] || [ "$edited" -eq 1 ]; then restore; fi' EXIT

tmp=$(mktemp)
sed "0,/^version = \".*\"/s//version = \"$version\"/" Cargo.toml >"$tmp"
mv "$tmp" Cargo.toml

tmp=$(mktemp)
awk -v ver="$version" '
    /^pkgver=/ { print "pkgver=" ver; next }
    /^pkgrel=/ { print "pkgrel=1"; next }
    { print }
' arch/PKGBUILD >"$tmp"
mv "$tmp" arch/PKGBUILD
edited=1

# Package version lives in Cargo.lock; --locked builds fail without this.
cargo update -p cosmic-printers --precise "$version"

./scripts/check-version.sh "$tag"

lock_version=$(
    awk '
        $0 == "name = \"cosmic-printers\"" { found=1; next }
        found && /^version = / {
            gsub(/"/, "", $3)
            print $3
            exit
        }
    ' Cargo.lock
)
if [ "$lock_version" != "$version" ]; then
    echo "Cargo.lock still at $lock_version, expected $version" >&2
    exit 1
fi

if [ "$dry_run" -eq 1 ]; then
    echo "dry-run: would commit, tag $tag, and push"
    git --no-pager diff --stat
    exit 0
fi

git add Cargo.toml arch/PKGBUILD Cargo.lock
git commit -m "chore(release): $version"
edited=0
git tag -a "$tag" -m "$tag"
trap - EXIT
git push origin main
git push origin "$tag"

echo "pushed $tag"
echo "watch: gh run watch --exit-status \$(gh run list --workflow=release-arch.yml --branch $tag --limit 1 --json databaseId -q '.[0].databaseId')"
echo "release: https://github.com/drmaas/cosmic-printers/releases/tag/$tag"
