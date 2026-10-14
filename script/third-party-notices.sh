#!/bin/sh
# Write target/THIRD-PARTY-NOTICES.txt: every crate in the release build with
# the licence text it's used under, which MIT, BSD, Apache and the rest ask to
# go with each copy of the binary. The release archives and Jig.app carry it.
#
# Needs cargo-about (cargo install cargo-about --locked --features cli). It
# fails on a dependency whose licence script/licenses/about.toml doesn't
# accept, so a new one is looked at before it ships.
set -eu

cd "$(dirname "$0")/.."
if ! cargo about --version >/dev/null 2>&1; then
    echo "cargo-about isn't installed: cargo install cargo-about --locked --features cli" >&2
    exit 1
fi
mkdir -p target
cargo about generate --locked --fail \
    -c script/licenses/about.toml \
    -o target/THIRD-PARTY-NOTICES.txt \
    script/licenses/notices.hbs
echo "Wrote target/THIRD-PARTY-NOTICES.txt"
