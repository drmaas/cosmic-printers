#!/bin/sh
# Stubbed checks for scripts/install-applet.sh. No network. No real pacman.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
script="$root/scripts/install-applet.sh"
failed=0

pass() {
    printf 'ok - %s\n' "$1"
}

fail() {
    printf 'not ok - %s\n' "$1" >&2
    failed=$((failed + 1))
}

assert_eq() {
    # assert_eq <label> <expected> <actual>
    if [ "$2" = "$3" ]; then
        pass "$1"
    else
        fail "$1 (expected '$2', got '$3')"
    fi
}

assert_contains() {
    # assert_contains <label> <needle> <haystack>
    case "$3" in
        *"$2"*) pass "$1" ;;
        *) fail "$1 (missing '$2')" ;;
    esac
}

assert_not_contains() {
    case "$3" in
        *"$2"*) fail "$1 (unexpected '$2')" ;;
        *) pass "$1" ;;
    esac
}

make_bin() {
    dir=$1
    mkdir -p "$dir"
    cat >"$dir/uname" <<'EOF'
#!/bin/sh
echo x86_64
EOF
    cat >"$dir/sudo" <<'EOF'
#!/bin/sh
# Record argv; drop leading sudo semantics by execing the rest.
printf '%s\n' "$*" >>"${INSTALL_APPLET_TEST_LOG}/sudo.log"
exec "$@"
EOF
    cat >"$dir/pacman" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"${INSTALL_APPLET_TEST_LOG}/pacman.log"
exit 0
EOF
    cat >"$dir/curl" <<'EOF'
#!/bin/sh
# Minimal GitHub API / asset stub. Honors -o like curl.
log="${INSTALL_APPLET_TEST_LOG}/curl.log"
printf '%s\n' "$*" >>"$log"

url=
out=
prev=
for arg in "$@"; do
    if [ "$prev" = "-o" ]; then
        out=$arg
        prev=
        continue
    fi
    case "$arg" in
        -o) prev=$arg ;;
        -*|--) ;;
        http*) url=$arg ;;
    esac
done

body=
case "$url" in
    */releases/latest)
        body='{"tag_name":"v0.1.4","assets":[{"name":"cosmic-printers-0.1.4-1-x86_64.pkg.tar.zst","browser_download_url":"https://github.com/drmaas/cosmic-printers/releases/download/v0.1.4/cosmic-printers-0.1.4-1-x86_64.pkg.tar.zst"}]}'
        ;;
    */releases/tags/v0.1.4)
        body='{"tag_name":"v0.1.4","assets":[{"name":"cosmic-printers-0.1.4-1-x86_64.pkg.tar.zst","browser_download_url":"https://github.com/drmaas/cosmic-printers/releases/download/v0.1.4/cosmic-printers-0.1.4-1-x86_64.pkg.tar.zst"}]}'
        ;;
    */releases/tags/v0.0.0)
        body='{"tag_name":"v0.0.0","assets":[]}'
        ;;
    */releases/tags/v9.9.9)
        # Compact single-line body (matches live GitHub API) with two assets.
        body='{"tag_name":"v9.9.9","assets":[{"name":"cosmic-printers-9.9.9-1-x86_64.pkg.tar.zst","browser_download_url":"https://github.com/drmaas/cosmic-printers/releases/download/v9.9.9/cosmic-printers-9.9.9-1-x86_64.pkg.tar.zst"},{"name":"cosmic-printers-9.9.9-2-x86_64.pkg.tar.zst","browser_download_url":"https://github.com/drmaas/cosmic-printers/releases/download/v9.9.9/cosmic-printers-9.9.9-2-x86_64.pkg.tar.zst"}]}'
        ;;
    *cosmic-printers-*-x86_64.pkg.tar.zst)
        body='pkg'
        ;;
    *)
        echo "unexpected curl url: $url" >&2
        exit 22
        ;;
esac

if [ -n "$out" ]; then
    printf '%s' "$body" >"$out"
else
    printf '%s' "$body"
fi
EOF
    chmod +x "$dir/uname" "$dir/sudo" "$dir/pacman" "$dir/curl"
}

run_script() {
    INSTALL_APPLET_TEST_LOG=$1
    export INSTALL_APPLET_TEST_LOG
    shift
    env PATH="$bin_dir:$PATH" \
        INSTALL_APPLET_FORCE_TTY=1 \
        "$script" "$@"
}

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT
bin_dir="$tmpdir/bin"
make_bin "$bin_dir"

# --- help exits 0 ---
out=$(run_script "$tmpdir/help" --help 2>&1) || status=$?
status=${status:-0}
assert_eq "help exit 0" "0" "$status"
assert_contains "help shows usage" "usage:" "$out"

# --- unknown flag exit 2 ---
status=0
out=$(run_script "$tmpdir/badflag" --bogus 2>&1) || status=$?
assert_eq "unknown flag exit 2" "2" "$status"

# --- bad version exit 2 ---
status=0
out=$(run_script "$tmpdir/badver" 1.2 2>&1) || status=$?
assert_eq "bad version exit 2" "2" "$status"

# --- dry-run latest: no pacman, prints plan ---
rm -f "$tmpdir/dry/pacman.log" 2>/dev/null || true
mkdir -p "$tmpdir/dry"
status=0
out=$(run_script "$tmpdir/dry" --dry-run latest 2>&1) || status=$?
assert_eq "dry-run latest exit 0" "0" "$status"
assert_contains "dry-run shows tag" "v0.1.4" "$out"
assert_contains "dry-run shows asset url" "cosmic-printers-0.1.4-1-x86_64.pkg.tar.zst" "$out"
if [ -f "$tmpdir/dry/pacman.log" ]; then
    fail "dry-run must not run pacman"
else
    pass "dry-run skips pacman"
fi
if [ -f "$tmpdir/dry/curl.log" ]; then
    assert_contains "dry-run curl uses -q first" "-q" "$(head -n1 "$tmpdir/dry/curl.log")"
else
    fail "dry-run should call curl for release metadata"
fi

# --- version normalize v0.1.4 ---
mkdir -p "$tmpdir/ver"
status=0
out=$(run_script "$tmpdir/ver" --dry-run v0.1.4 2>&1) || status=$?
assert_eq "version v0.1.4 exit 0" "0" "$status"
assert_contains "version path hits tags API" "releases/tags/v0.1.4" "$(cat "$tmpdir/ver/curl.log")"

# --- bare 0.1.4 same ---
mkdir -p "$tmpdir/ver2"
status=0
out=$(run_script "$tmpdir/ver2" --dry-run 0.1.4 2>&1) || status=$?
assert_eq "version 0.1.4 exit 0" "0" "$status"
assert_contains "bare version hits tags API" "releases/tags/v0.1.4" "$(cat "$tmpdir/ver2/curl.log")"

# --- missing asset ---
mkdir -p "$tmpdir/noasset"
status=0
out=$(run_script "$tmpdir/noasset" --dry-run v0.0.0 2>&1) || status=$?
assert_eq "missing asset non-zero" "1" "$status"
assert_not_contains "missing asset no pacman file" "pacman" "$(ls "$tmpdir/noasset")"

# --- two assets ---
mkdir -p "$tmpdir/two"
status=0
out=$(run_script "$tmpdir/two" --dry-run v9.9.9 2>&1) || status=$?
assert_eq "two assets non-zero" "1" "$status"

# --- install with --yes runs pacman -U --noconfirm ---
mkdir -p "$tmpdir/yes"
status=0
out=$(run_script "$tmpdir/yes" --yes 0.1.4 2>&1) || status=$?
assert_eq "install --yes exit 0" "0" "$status"
assert_contains "pacman -U" "-U" "$(cat "$tmpdir/yes/pacman.log")"
assert_contains "pacman --noconfirm" "--noconfirm" "$(cat "$tmpdir/yes/pacman.log")"
assert_contains "sudo used" "pacman" "$(cat "$tmpdir/yes/sudo.log")"

# --- wrong arch ---
cat >"$bin_dir/uname" <<'EOF'
#!/bin/sh
echo aarch64
EOF
chmod +x "$bin_dir/uname"
mkdir -p "$tmpdir/arch"
status=0
out=$(run_script "$tmpdir/arch" --dry-run latest 2>&1) || status=$?
assert_eq "wrong arch non-zero" "1" "$status"

if [ "$failed" -ne 0 ]; then
    echo "$failed check(s) failed" >&2
    exit 1
fi
echo "all checks passed"
