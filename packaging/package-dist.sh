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
# Usage: packaging/package-dist.sh [--linux-only]
#   --linux-only  skip the Windows cross-build and its two archives (MinGW and the Windows
#                 target are then not needed). The release workflow uses this and builds
#                 the Windows archives natively with packaging/package-windows.ps1.

set -euo pipefail

BUILD_WINDOWS=true
for argument in "$@"; do
    case "$argument" in
        --linux-only) BUILD_WINDOWS=false ;;
        *) echo "Unknown option: $argument (expected --linux-only)" >&2; exit 2 ;;
    esac
done

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
for tool in cargo cargo-zigbuild zig objdump tar sha256sum file; do
    require "$tool"
done
required_targets=("$MUSL_TARGET")
if $BUILD_WINDOWS; then
    for tool in x86_64-w64-mingw32-gcc x86_64-w64-mingw32-objdump zip; do
        require "$tool"
    done
    required_targets+=("$WINDOWS_TARGET")
fi
for target in "${required_targets[@]}"; do
    rustup target list --installed | grep -x "$target" >/dev/null \
        || { echo "Missing Rust target: run 'rustup target add $target'" >&2; exit 1; }
done

cd "$ROOT"

if $BUILD_WINDOWS; then
    echo "== Building Windows (GUI + CLI) =="
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
        cargo build --release --locked --target "$WINDOWS_TARGET"
fi

echo "== Building Linux GUI + CLI against glibc $GLIBC_BASELINE =="
cargo zigbuild --release --locked --target "$LINUX_TARGET.$GLIBC_BASELINE"

echo "== Building static Linux CLI (musl) =="
cargo zigbuild --release --locked --no-default-features --bin link-resolver-cli --target "$MUSL_TARGET"

WINDOWS_OUT="target/$WINDOWS_TARGET/release"
LINUX_OUT="target/$LINUX_TARGET/release"
MUSL_OUT="target/$MUSL_TARGET/release"

echo "== Checking build outputs =="
subsystem() { x86_64-w64-mingw32-objdump -p "$1" | awk '/^Subsystem/ {print $0}'; }
if $BUILD_WINDOWS; then
    subsystem "$WINDOWS_OUT/link-resolver.exe" | grep "Windows GUI" >/dev/null \
        || { echo "link-resolver.exe is not a GUI-subsystem executable" >&2; exit 1; }
    subsystem "$WINDOWS_OUT/link-resolver-cli.exe" | grep "Windows CUI" >/dev/null \
        || { echo "link-resolver-cli.exe is not a console executable" >&2; exit 1; }
fi
highest_glibc() { objdump -T "$1" | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1 | sed 's/GLIBC_//'; }
for binary in "$LINUX_OUT/link-resolver" "$LINUX_OUT/link-resolver-cli"; do
    needed=$(highest_glibc "$binary")
    if [ "$(printf '%s\n%s\n' "$needed" "$GLIBC_BASELINE" | sort -V | tail -1)" != "$GLIBC_BASELINE" ]; then
        echo "$binary needs glibc $needed, above the $GLIBC_BASELINE baseline" >&2; exit 1
    fi
done
file "$MUSL_OUT/link-resolver-cli" | grep "static" >/dev/null \
    || { echo "musl CLI is not statically linked" >&2; exit 1; }
# A real run of the static CLI, touching no network: it must report Cargo.toml's version.
expected_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
reported_version=$("$MUSL_OUT/link-resolver-cli" --version)
[ "$reported_version" = "Link Resolver $expected_version" ] \
    || { echo "link-resolver-cli --version printed '$reported_version', expected 'Link Resolver $expected_version'" >&2; exit 1; }
echo "$reported_version"
if $BUILD_WINDOWS; then
    echo "Windows GUI imports:"
    x86_64-w64-mingw32-objdump -p "$WINDOWS_OUT/link-resolver.exe" | awk '/DLL Name/ {print "  " tolower($3)}' | sort -u
fi

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
if $BUILD_WINDOWS; then
    stage gui-windows-x64 QUICKSTART.txt \
        "$WINDOWS_OUT/link-resolver.exe:link-resolver.exe" \
        "$WINDOWS_OUT/link-resolver-cli.exe:link-resolver-cli.exe"
    stage windows-x64 packaging/QUICKSTART-terminal.txt \
        "$WINDOWS_OUT/link-resolver-cli.exe:link-resolver.exe"
fi
stage gui-linux-x64 QUICKSTART.txt \
    "$LINUX_OUT/link-resolver:link-resolver" \
    "$LINUX_OUT/link-resolver-cli:link-resolver-cli"
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
if $BUILD_WINDOWS; then
    archive gui-windows-x64 link-resolver-gui-windows-x64.zip
    archive windows-x64 link-resolver-windows-x64.zip
fi
archive gui-linux-x64 link-resolver-gui-linux-x64.tar.gz
archive linux-x64 link-resolver-linux-x64.tar.gz
rm -rf "$STAGE"

(cd "$DIST" && find . -maxdepth 2 -type f \( -name 'link-resolver*' -o -name '*.tar.gz' -o -name '*.zip' \) \
    | sed 's,^\./,,' | sort | xargs sha256sum > SHA256SUMS)
echo "== Done =="
cat "$DIST/SHA256SUMS"
