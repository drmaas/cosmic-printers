name := 'cosmic-printers'
appid := 'io.github.drmaas.CosmicPrinters'
rootdir := ''
prefix := '/usr'

base-dir := absolute_path(clean(rootdir / prefix))
cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
bin-dst := base-dir / 'bin' / name
desktop-dst := base-dir / 'share' / 'applications' / appid + '.desktop'
metainfo-dst := base-dir / 'share' / 'metainfo' / appid + '.metainfo.xml'
icon-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'scalable' / 'apps' / appid + '.svg'

default: build-release

clean:
    cargo clean

build-debug *args:
    cargo build {{ args }}

build-release *args: (build-debug '--release' args)

check *args:
    cargo clippy --all-targets --locked {{ args }} -- -D warnings

fmt:
    cargo fmt --all -- --check

test:
    cargo test --locked

run *args:
    env RUST_BACKTRACE=full cargo run --release {{ args }}

install: build-release
    install -Dm0755 {{ cargo-target-dir / 'release' / name }} {{ bin-dst }}
    install -Dm0644 resources/{{ appid }}.desktop {{ desktop-dst }}
    install -Dm0644 resources/{{ appid }}.metainfo.xml {{ metainfo-dst }}
    install -Dm0644 resources/icons/hicolor/scalable/apps/{{ appid }}.svg {{ icon-dst }}

uninstall:
    rm -f {{ bin-dst }} {{ desktop-dst }} {{ metainfo-dst }} {{ icon-dst }}
