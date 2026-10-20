#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(CDPATH= cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly PROJECT_ROOT="$(CDPATH= cd -P -- "$SCRIPT_DIR/.." && pwd)"
readonly INFO_PLIST="$PROJECT_ROOT/macos/Info.plist"
readonly APP_NAME="MacPlayer.app"
readonly PRODUCT_BINARY="macplayer"
readonly CARGO_BIN="${CARGO:-cargo}"

if [[ "$#" -ne 1 ]]; then
    printf 'Usage: %s <arm64|x86_64>\n' "$0" >&2
    exit 2
fi

case "$1" in
    arm64)
        readonly TARGET_ARCH="arm64"
        readonly RUST_TARGET="aarch64-apple-darwin"
        ;;
    x86_64)
        readonly TARGET_ARCH="x86_64"
        readonly RUST_TARGET="x86_64-apple-darwin"
        ;;
    *)
        printf 'Usage: %s <arm64|x86_64>\n' "$0" >&2
        printf 'package-macos: error: unsupported architecture: %s\n' "$1" >&2
        exit 2
        ;;
esac

dist_input="${MACPLAYER_DIST_DIR:-dist/macos-$TARGET_ARCH}"
case "$dist_input" in
    /*) readonly DIST_DIR="$dist_input" ;;
    *) readonly DIST_DIR="$PROJECT_ROOT/$dist_input" ;;
esac

readonly APP_BUNDLE="$DIST_DIR/$APP_NAME"
readonly APP_BINARY="$APP_BUNDLE/Contents/MacOS/$PRODUCT_BINARY"
readonly FRAMEWORKS_DIR="$APP_BUNDLE/Contents/Frameworks"
readonly APP_ICON="$APP_BUNDLE/Contents/Resources/MacPlayer.icns"
readonly APP_FRAMEWORK="$FRAMEWORKS_DIR/libmpv.2.dylib"
readonly ZIP_ARCHIVE="$DIST_DIR/MacPlayer-macos-$TARGET_ARCH.zip"
readonly DMG_IMAGE="$DIST_DIR/MacPlayer-macos-$TARGET_ARCH.dmg"

die() {
    printf 'package-macos: error: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

is_system_dependency() {
    case "$1" in
        /System/Library/*|/usr/lib/*) return 0 ;;
        *) return 1 ;;
    esac
}

dylib_dependencies() {
    otool -L "$1" | sed -n 's/^[[:space:]]*\([^[:space:]]*\.dylib\) (.*$/\1/p'
}

dylib_rpaths() {
    otool -l "$1" | awk '
        $1 == "cmd" { in_rpath = ($2 == "LC_RPATH"); next }
        in_rpath && $1 == "path" { print $2; in_rpath = 0 }
    '
}

resolve_rpath() {
    local owner="$1"
    local rpath="$2"
    case "$rpath" in
        @loader_path/*) printf '%s\n' "$(dirname "$owner")/${rpath#@loader_path/}" ;;
        @executable_path/*) printf '%s\n' "$APP_BUNDLE/Contents/MacOS/${rpath#@executable_path/}" ;;
        *) printf '%s\n' "$rpath" ;;
    esac
}

resolve_dependency() {
    local owner="$1"
    local dependency="$2"
    local candidate
    local relative="${dependency##*/}"
    local rpath

    case "$dependency" in
        /*)
            [[ -f "$dependency" ]] && printf '%s\n' "$dependency" && return 0
            ;;
        @loader_path/*)
            candidate="$(dirname "$owner")/${dependency#@loader_path/}"
            [[ -f "$candidate" ]] && printf '%s\n' "$candidate" && return 0
            ;;
        @executable_path/*)
            candidate="$APP_BUNDLE/Contents/MacOS/${dependency#@executable_path/}"
            [[ -f "$candidate" ]] && printf '%s\n' "$candidate" && return 0
            ;;
        @rpath/*)
            relative="${dependency#@rpath/}"
            while IFS= read -r rpath; do
                [[ -n "$rpath" ]] || continue
                rpath="$(resolve_rpath "$owner" "$rpath")"
                candidate="$rpath/$relative"
                [[ -f "$candidate" ]] && printf '%s\n' "$candidate" && return 0
            done < <(dylib_rpaths "$owner")
            ;;
    esac

    if [[ -n "${BREW_PREFIX:-}" ]]; then
        for candidate in \
            "$BREW_PREFIX/lib/$relative" \
            "$BREW_PREFIX/opt"/*/lib/"$relative"; do
            [[ -f "$candidate" ]] && printf '%s\n' "$candidate" && return 0
        done
    fi
    return 1
}

bundle_dependency() {
    local owner="$1"
    local dependency="$2"
    local source
    local name
    local destination

    is_system_dependency "$dependency" && return 0
    source="$(resolve_dependency "$owner" "$dependency" || true)"
    [[ -n "$source" ]] || die "cannot resolve non-system dependency $dependency required by $owner"
    name="$(basename "$source")"
    destination="$FRAMEWORKS_DIR/$name"
    [[ "$owner" == "$destination" ]] && return 0

    verify_architecture "$source" "dependency"
    install_name_tool -change "$dependency" "@rpath/$name" "$owner"
    if [[ ! -e "$destination" ]]; then
        cp "$source" "$destination"
        chmod 755 "$destination"
        install_name_tool -id "@rpath/$name" "$destination"
        install_name_tool -add_rpath "@loader_path" "$destination" 2>/dev/null || true
        while IFS= read -r child; do
            [[ -n "$child" ]] || continue
            bundle_dependency "$destination" "$child"
        done < <(dylib_dependencies "$destination")
    fi
}

bundle_runtime() {
    cp "$LIBMPV_SOURCE" "$APP_FRAMEWORK"
    chmod 755 "$APP_FRAMEWORK"
    install_name_tool -id "@rpath/libmpv.2.dylib" "$APP_FRAMEWORK"
    install_name_tool -add_rpath "@loader_path" "$APP_FRAMEWORK" 2>/dev/null || true
    while IFS= read -r dependency; do
        [[ -n "$dependency" ]] || continue
        bundle_dependency "$APP_FRAMEWORK" "$dependency"
    done < <(dylib_dependencies "$APP_FRAMEWORK")
}

verify_architecture() {
    local path="$1"
    local description="$2"
    local architectures

    architectures="$(lipo -archs "$path" 2>/dev/null || true)"
    [[ "$architectures" == "$TARGET_ARCH" ]] || die \
        "$description must contain exactly one $TARGET_ARCH slice (found: ${architectures:-none}): $path"
}

has_target_architecture() {
    local path="$1"
    [[ "$(lipo -archs "$path" 2>/dev/null || true)" == "$TARGET_ARCH" ]]
}

resolve_libmpv() {
    local candidate
    local brew_mpv_prefix
    local brew_prefix
    local -a candidates=()

    if [[ -n "${MACPLAYER_LIBMPV:-}" ]]; then
        LIBMPV_SOURCE="$MACPLAYER_LIBMPV"
        case "$LIBMPV_SOURCE" in
            /*) ;;
            *) LIBMPV_SOURCE="$PWD/$LIBMPV_SOURCE" ;;
        esac
        [[ -f "$LIBMPV_SOURCE" && -r "$LIBMPV_SOURCE" ]] || die \
            "MACPLAYER_LIBMPV is not a readable file: $MACPLAYER_LIBMPV"
        return
    fi

    if command -v brew >/dev/null 2>&1; then
        brew_mpv_prefix="$(brew --prefix mpv 2>/dev/null || true)"
        if [[ -n "$brew_mpv_prefix" ]]; then
            candidates+=(
                "$brew_mpv_prefix/lib/libmpv.2.dylib"
                "$brew_mpv_prefix/lib/libmpv.dylib"
            )
        fi

        brew_prefix="$(brew --prefix 2>/dev/null || true)"
        if [[ -n "$brew_prefix" ]]; then
            candidates+=(
                "$brew_prefix/lib/libmpv.2.dylib"
                "$brew_prefix/lib/libmpv.dylib"
            )
        fi
    fi

    candidates+=(
        "/opt/homebrew/opt/mpv/lib/libmpv.2.dylib"
        "/opt/homebrew/opt/mpv/lib/libmpv.dylib"
        "/opt/homebrew/lib/libmpv.2.dylib"
        "/opt/homebrew/lib/libmpv.dylib"
        "/usr/local/opt/mpv/lib/libmpv.2.dylib"
        "/usr/local/opt/mpv/lib/libmpv.dylib"
        "/usr/local/lib/libmpv.2.dylib"
        "/usr/local/lib/libmpv.dylib"
    )

    for candidate in "${candidates[@]}"; do
        if [[ -f "$candidate" && -r "$candidate" ]] && has_target_architecture "$candidate"; then
            LIBMPV_SOURCE="$candidate"
            return
        fi
    done

    die "a matching $TARGET_ARCH libmpv was not found; set MACPLAYER_LIBMPV to a readable $TARGET_ARCH libmpv .dylib or install a matching mpv with Homebrew"
}

require_command uname
[[ "$(uname -s)" == "Darwin" ]] || die "macOS packaging must run on macOS (Darwin)"

require_command mkdir
require_command cp
require_command rm
require_command chmod
require_command plutil
require_command ditto
require_command otool
require_command install_name_tool
require_command lipo
require_command swift
require_command iconutil
require_command mktemp
require_command "$CARGO_BIN"

[[ -f "$INFO_PLIST" ]] || die "missing app bundle metadata: $INFO_PLIST"

plutil -lint "$INFO_PLIST" >/dev/null || die "invalid app bundle metadata: $INFO_PLIST"

LIBMPV_SOURCE=""
BREW_PREFIX=""
if command -v brew >/dev/null 2>&1; then
    BREW_PREFIX="$(brew --prefix 2>/dev/null || true)"
fi

HOST_ARCH="$(uname -m)"
if [[ "$HOST_ARCH" != "$TARGET_ARCH" ]]; then
    printf 'package-macos: cross-target build: host=%s target=%s\n' "$HOST_ARCH" "$TARGET_ARCH" >&2
    if [[ "$TARGET_ARCH" == "x86_64" && "$HOST_ARCH" == "arm64" ]]; then
        printf 'package-macos: an arm64 Homebrew libmpv cannot be used for this Intel package; provide an x86_64 libmpv with MACPLAYER_LIBMPV\n' >&2
    fi
fi

resolve_libmpv
verify_architecture "$LIBMPV_SOURCE" "libmpv"

cd "$PROJECT_ROOT"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-13.0}"
"$CARGO_BIN" build --release --offline --locked --target "$RUST_TARGET"

release_binary="$PROJECT_ROOT/target/$RUST_TARGET/release/$PRODUCT_BINARY"
[[ -x "$release_binary" ]] || die "release binary was not produced: $release_binary"
verify_architecture "$release_binary" "release binary"

rm -rf "$APP_BUNDLE"
rm -f "$ZIP_ARCHIVE" "$DMG_IMAGE"
mkdir -p \
    "$APP_BUNDLE/Contents/MacOS" \
    "$APP_BUNDLE/Contents/Frameworks" \
    "$APP_BUNDLE/Contents/Resources"

cp "$release_binary" "$APP_BINARY"
cp "$INFO_PLIST" "$APP_BUNDLE/Contents/Info.plist"
chmod 755 "$APP_BINARY"

iconset_stage=""
iconset_dir=""
dmg_stage=""
cleanup() {
    if [[ -n "$iconset_stage" && -d "$iconset_stage" ]]; then
        rm -rf "$iconset_stage"
    fi
    if [[ -n "$dmg_stage" && -d "$dmg_stage" ]]; then
        rm -rf "$dmg_stage"
    fi
}
trap cleanup EXIT

iconset_stage="$(mktemp -d "${TMPDIR:-/tmp}/macplayer-icon.XXXXXX")"
iconset_dir="$iconset_stage/MacPlayer.iconset"
mkdir -p "$iconset_dir"
swift "$PROJECT_ROOT/macos/generate-icon.swift" "$iconset_dir"
iconutil -c icns "$iconset_dir" -o "$APP_ICON"

# The Rust player loads libmpv dynamically. Add the bundle's Frameworks directory
# to dyld's runtime search paths without changing the release binary in target/.
install_name_tool -add_rpath "@executable_path/../Frameworks" "$APP_BINARY" 2>/dev/null || true
bundle_runtime

# Ad-hoc signing makes a locally built bundle launch cleanly after its Mach-O
# load commands were rewritten. A developer can replace it with Developer ID
# signing before publishing.
if command -v codesign >/dev/null 2>&1; then
    codesign --force --deep --sign - "$APP_BUNDLE" >/dev/null
fi

# Archive only the app bundle, not the build tree or another generated artifact.
COPYFILE_DISABLE=1 ditto -c -k --keepParent --norsrc "$APP_BUNDLE" "$ZIP_ARCHIVE"

if command -v hdiutil >/dev/null 2>&1; then
    dmg_stage="$(mktemp -d "${TMPDIR:-/tmp}/macplayer-dmg.XXXXXX")"
    cp -R "$APP_BUNDLE" "$dmg_stage/$APP_NAME"
    hdiutil create \
        -ov \
        -format UDZO \
        -volname "MacPlayer" \
        -srcfolder "$dmg_stage" \
        "$DMG_IMAGE"
else
    printf 'package-macos: hdiutil not found; skipped DMG creation\n' >&2
fi

printf 'package-macos: app  %s\n' "$APP_BUNDLE"
printf 'package-macos: zip  %s\n' "$ZIP_ARCHIVE"
if [[ -f "$DMG_IMAGE" ]]; then
    printf 'package-macos: dmg  %s\n' "$DMG_IMAGE"
fi
