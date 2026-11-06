#!/usr/bin/env bash
# Build the distributable archives in dist/ for Windows and Linux x86-64.
#
# Produces four archives, each unpacking to a link-resolver/ folder:
#   link-resolver-gui-windows-x64.zip     desktop app + CLI (Windows 10/11)
#   link-resolver-gui-linux-x64.tar.gz    desktop app + CLI (glibc >= GLIBC_BASELINE)
#   link-resolver-windows-x64.zip         terminal-only edition
#   link-resolver-linux-x64.tar.gz        terminal-only edition, fully static (musl)
# plus unpacked copies and SHA256SUMS. Existing dist/ contents are replaced.
#
# Requirements on the build machine (Linux):
#   rustup target add x86_64-pc-windows-gnu x86_64-unknown-linux-musl
#   MinGW (x86_64-w64-mingw32-gcc, x86_64-w64-mingw32-objdump), zig, cargo-zigbuild, zip
#
# Usage: packaging/package-dist.sh

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DIST="$ROOT/dist"
STAGE="$ROOT/target/dist-staging"
GLIBC_BASELINE=2.28
WINDOWS_TARGET=x86_64-pc-windows-gnu
LINUX_TARGET=x86_64-unknown-linux-gnu
MUSL_TARGET=x86_64-unknown-linux-musl

require() {
    command -v "$1" >/dev/null 2>&1 || { echo "Missing required tool: $1" >&2; exit 1; }
}
for tool in cargo cargo-zigbuild zig x86_64-w64-mingw32-gcc x86_64-w64-mingw32-objdump objdump zip tar sha256sum; do
    require "$tool"
done
for target in "$WINDOWS_TARGET" "$MUSL_TARGET"; do
    rustup target list --installed | grep -qx "$target" \
        || { echo "Missing Rust target: run 'rustup target add $target'" >&2; exit 1; }
done

cd "$ROOT"

echo "== Building Windows (GUI + CLI) =="
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
    cargo build --release --locked --target "$WINDOWS_TARGET"

echo "== Building Linux GUI + CLI against glibc $GLIBC_BASELINE =="
cargo zigbuild --release --locked --target "$LINUX_TARGET.$GLIBC_BASELINE"

echo "== Building static Linux CLI (musl) =="
cargo zigbuild --release --locked --no-default-features --bin link-resolver-cli --target "$MUSL_TARGET"

WINDOWS_OUT="target/$WINDOWS_TARGET/release"
LINUX_OUT="target/$LINUX_TARGET/release"
MUSL_OUT="target/$MUSL_TARGET/release"

echo "== Checking build outputs =="
subsystem() { x86_64-w64-mingw32-objdump -p "$1" | awk '/^Subsystem/ {print $0}'; }
subsystem "$WINDOWS_OUT/link-resolver.exe" | grep -q "Windows GUI" \
    || { echo "link-resolver.exe is not a GUI-subsystem executable" >&2; exit 1; }
subsystem "$WINDOWS_OUT/link-resolver-cli.exe" | grep -q "Windows CUI" \
    || { echo "link-resolver-cli.exe is not a console executable" >&2; exit 1; }
highest_glibc() { objdump -T "$1" | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1 | sed 's/GLIBC_//'; }
for binary in "$LINUX_OUT/link-resolver" "$LINUX_OUT/link-resolver-cli"; do
    needed=$(highest_glibc "$binary")
    if [ "$(printf '%s\n%s\n' "$needed" "$GLIBC_BASELINE" | sort -V | tail -1)" != "$GLIBC_BASELINE" ]; then
        echo "$binary needs glibc $needed, above the $GLIBC_BASELINE baseline" >&2; exit 1
    fi
done
file "$MUSL_OUT/link-resolver-cli" | grep -q "static" \
    || { echo "musl CLI is not statically linked" >&2; exit 1; }
echo "Windows GUI imports:"
x86_64-w64-mingw32-objdump -p "$WINDOWS_OUT/link-resolver.exe" | awk '/DLL Name/ {print "  " tolower($3)}' | sort -u

echo "== Staging =="
rm -rf "$DIST" "$STAGE"
mkdir -p "$DIST" "$STAGE"

# stage <folder> <quickstart> <source>:<name>...
stage() {
    local folder="$DIST/$1" quickstart="$2"
    shift 2
    mkdir -p "$folder"
    cp "$quickstart" "$folder/QUICKSTART.txt"
    cp examples/urls.txt "$folder/urls.txt"
    cp examples/links.csv "$folder/links.csv"
    local pair
    for pair in "$@"; do
        cp "${pair%%:*}" "$folder/${pair##*:}"
    done
}
stage gui-windows-x64 QUICKSTART.txt \
    "$WINDOWS_OUT/link-resolver.exe:link-resolver.exe" \
    "$WINDOWS_OUT/link-resolver-cli.exe:link-resolver-cli.exe"
stage gui-linux-x64 QUICKSTART.txt \
    "$LINUX_OUT/link-resolver:link-resolver" \
    "$LINUX_OUT/link-resolver-cli:link-resolver-cli"
stage windows-x64 packaging/QUICKSTART-terminal.txt \
    "$WINDOWS_OUT/link-resolver-cli.exe:link-resolver.exe"
stage linux-x64 packaging/QUICKSTART-terminal.txt \
    "$MUSL_OUT/link-resolver-cli:link-resolver"

# archive <folder> <archive-name>: the archive unpacks to link-resolver/.
archive() {
    rm -rf "$STAGE/link-resolver"
    cp -r "$DIST/$1" "$STAGE/link-resolver"
    case "$2" in
        *.tar.gz) tar -C "$STAGE" -czf "$DIST/$2" link-resolver ;;
        *.zip) (cd "$STAGE" && zip -qr "$DIST/$2" link-resolver) ;;
    esac
}
archive gui-windows-x64 link-resolver-gui-windows-x64.zip
archive gui-linux-x64 link-resolver-gui-linux-x64.tar.gz
archive windows-x64 link-resolver-windows-x64.zip
archive linux-x64 link-resolver-linux-x64.tar.gz
rm -rf "$STAGE"

(cd "$DIST" && sha256sum */link-resolver* ./*.tar.gz ./*.zip | sed 's,^\([^ ]*  \)\./,\1,' > SHA256SUMS)
echo "== Done =="
cat "$DIST/SHA256SUMS"
