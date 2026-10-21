#!/usr/bin/env bash

# Merge two single-architecture MacPlayer.app bundles into a Universal bundle.
#
# Usage:
#   package-macos-universal.sh [--check] <arm64-app> <x86_64-app> <output-dir>
#
# The normal mode writes these files in <output-dir>:
#   MacPlayer.app
#   MacPlayer-macos-universal.zip
#   MacPlayer-macos-universal.dmg  (when hdiutil is available)
#
# --check validates the input bundles, Mach-O slices, framework inventories, and
# output names without creating or modifying anything.
# Existing output names are refused rather than overwritten. Framework dylibs
# must be regular files (symlinked dylibs are rejected explicitly).

set -euo pipefail

readonly APP_NAME="MacPlayer.app"
readonly UNIVERSAL_ZIP="MacPlayer-macos-universal.zip"
readonly UNIVERSAL_DMG="MacPlayer-macos-universal.dmg"

CHECK_ONLY=0
ARM_APP=""
X86_APP=""
OUTPUT_DIR=""
ARM_EXECUTABLE=""
X86_EXECUTABLE=""
WORK_DIR=""
MERGE_TMP=""
PUBLISHING=0
PUBLISHED_PATHS=()

usage() {
    printf 'Usage: %s [--check] <arm64-app> <x86_64-app> <output-dir>\n' "$0" >&2
    printf '\nMerge MacPlayer.app bundles and create a Universal app, zip, and optional DMG.\n' >&2
    printf 'The output app is <output-dir>/%s.\n' "$APP_NAME" >&2
    printf 'Existing output names are refused; use an empty output directory.\n' >&2
}

die() {
    printf 'package-macos-universal: error: %s\n' "$*" >&2
    exit 1
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

absolute_path() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        *) printf '%s/%s\n' "$PWD" "$1" ;;
    esac
}

cleanup() {
    local status=$?
    local path

    if (( status != 0 && PUBLISHING == 1 )); then
        for path in "${PUBLISHED_PATHS[@]}"; do
            rm -rf "$path" 2>/dev/null || true
        done
    fi

    if [[ -n "$WORK_DIR" && -d "$WORK_DIR" ]]; then
        rm -rf "$WORK_DIR" 2>/dev/null || true
    fi
    exit "$status"
}

trap cleanup EXIT

parse_arguments() {
    if [[ "$#" -eq 4 && "$1" == "--check" ]]; then
        CHECK_ONLY=1
        shift
    elif [[ "$#" -ne 3 ]]; then
        usage
        exit 2
    fi

    if [[ "$#" -ne 3 ]]; then
        usage
        exit 2
    fi

    ARM_APP="$(absolute_path "$1")"
    X86_APP="$(absolute_path "$2")"
    OUTPUT_DIR="$(absolute_path "$3")"
}

verify_architecture() {
    local path="$1"
    local expected="$2"
    local description="$3"
    local architectures

    [[ -f "$path" ]] || die "$description is missing: $path"
    architectures="$(lipo -archs "$path" 2>/dev/null || true)"
    if [[ "$expected" == "arm64 x86_64" ]]; then
        [[ "$architectures" == "arm64 x86_64" || "$architectures" == "x86_64 arm64" ]] || die \
            "$description must contain exactly arm64 and x86_64 slices (found: ${architectures:-none}): $path"
    else
        [[ "$architectures" == "$expected" ]] || die \
            "$description must contain exactly one $expected slice (found: ${architectures:-none}): $path"
    fi
}

bundle_executable() {
    local app="$1"
    local plist="$app/Contents/Info.plist"
    local executable

    [[ -d "$app" && "$app" == *.app ]] || die "not an app bundle: $app"
    [[ -d "$app/Contents" ]] || die "app bundle has no Contents directory: $app"
    [[ -f "$plist" ]] || die "app bundle has no Info.plist: $app"
    plutil -lint "$plist" >/dev/null || die "invalid app bundle metadata: $plist"

    executable="$(plutil -extract CFBundleExecutable raw -o - "$plist" 2>/dev/null || true)"
    [[ -n "$executable" && "$executable" != */* ]] || die \
        "Info.plist has no usable CFBundleExecutable: $plist"
    printf '%s/Contents/MacOS/%s\n' "$app" "$executable"
}

framework_inventory() {
    local app="$1"
    local frameworks="$app/Contents/Frameworks"
    local path

    [[ -d "$frameworks" ]] || die "app bundle has no Contents/Frameworks directory: $app"

    # A symlinked dylib is rejected rather than replaced accidentally. The
    # project packages concrete dylib files, and this keeps the merge atomic
    # and the inventory comparison unambiguous.
    if [[ -n "$(find "$frameworks" -type l -name '*.dylib' -print)" ]]; then
        die "symbolic-link dylibs are not supported in the framework directory: $frameworks"
    fi

    while IFS= read -r path; do
        [[ -n "$path" ]] || continue
        printf '%s\n' "${path#"$app/"}"
    done < <(find "$frameworks" -type f -name '*.dylib' -print) \
        | LC_ALL=C sort
}

framework_inventory_union() {
    local arm_app="$1"
    local x86_app="$2"
    local arm_inventory
    local x86_inventory

    arm_inventory="$(framework_inventory "$arm_app")"
    x86_inventory="$(framework_inventory "$x86_app")"
    printf '%s\n%s\n' "$arm_inventory" "$x86_inventory" \
        | awk 'NF' \
        | LC_ALL=C sort -u
}

validate_bundle() {
    local app="$1"
    local expected_arch="$2"
    local description="$3"
    local executable
    local relative
    local inventory

    executable="$(bundle_executable "$app")"
    verify_architecture "$executable" "$expected_arch" "$description executable"

    inventory="$(framework_inventory "$app")"
    while IFS= read -r relative; do
        [[ -n "$relative" ]] || continue
        verify_architecture "$app/$relative" "$expected_arch" \
            "$description framework dylib"
    done <<< "$inventory"

    printf '%s\n' "$executable"
}

check_output_paths() {
    local path

    if [[ -e "$OUTPUT_DIR" && ! -d "$OUTPUT_DIR" ]]; then
        die "output path is not a directory: $OUTPUT_DIR"
    fi
    if [[ ! -d "$OUTPUT_DIR" && ! -d "$(dirname "$OUTPUT_DIR")" ]]; then
        die "output directory parent does not exist: $(dirname "$OUTPUT_DIR")"
    fi

    for path in \
        "$OUTPUT_DIR/$APP_NAME" \
        "$OUTPUT_DIR/$UNIVERSAL_ZIP" \
        "$OUTPUT_DIR/$UNIVERSAL_DMG"; do
        [[ ! -e "$path" && ! -L "$path" ]] || die \
            "refusing to overwrite existing output: $path"
    done
}

validate_inputs() {
    [[ -d "$ARM_APP" ]] || die "arm64 app bundle does not exist: $ARM_APP"
    [[ -d "$X86_APP" ]] || die "x86_64 app bundle does not exist: $X86_APP"

    ARM_APP="$(CDPATH= cd -P -- "$ARM_APP" && pwd -P)"
    X86_APP="$(CDPATH= cd -P -- "$X86_APP" && pwd -P)"
    if [[ -d "$OUTPUT_DIR" ]]; then
        OUTPUT_DIR="$(CDPATH= cd -P -- "$OUTPUT_DIR" && pwd -P)"
    elif [[ -d "$(dirname "$OUTPUT_DIR")" ]]; then
        OUTPUT_DIR="$(CDPATH= cd -P -- "$(dirname "$OUTPUT_DIR")" && pwd -P)/$(basename "$OUTPUT_DIR")"
    fi

    case "$OUTPUT_DIR/" in
        "$ARM_APP/"*|"$X86_APP/"*)
            die "output directory must not be inside either input app bundle: $OUTPUT_DIR"
            ;;
    esac

    ARM_EXECUTABLE="$(validate_bundle "$ARM_APP" arm64 arm64)"
    X86_EXECUTABLE="$(validate_bundle "$X86_APP" x86_64 x86_64)"

    [[ "$(basename "$ARM_EXECUTABLE")" == "$(basename "$X86_EXECUTABLE")" ]] || die \
        "app executable names differ: $(basename "$ARM_EXECUTABLE") vs $(basename "$X86_EXECUTABLE")"
}

merge_macho() {
    local relative="$1"
    local arm_path="$ARM_APP/$relative"
    local x86_path="$X86_APP/$relative"
    local staged_path="$WORK_DIR/app/$APP_NAME/$relative"
    local output_path="$MERGE_TMP/$relative"

    mkdir -p "$(dirname "$output_path")"
    lipo -create "$arm_path" "$x86_path" -output "$output_path" || die \
        "failed to merge Mach-O files: $relative"
    chmod 755 "$output_path"
    mv -f "$output_path" "$staged_path"
    verify_architecture "$staged_path" "arm64 x86_64" "Universal Mach-O file"
}

merge_framework() {
    local relative="$1"
    local arm_path="$ARM_APP/$relative"
    local x86_path="$X86_APP/$relative"
    local staged_path="$WORK_DIR/app/$APP_NAME/$relative"

    if [[ -f "$arm_path" && -f "$x86_path" ]]; then
        merge_macho "$relative"
    elif [[ -f "$arm_path" ]]; then
        verify_architecture "$arm_path" "arm64" "arm64-only framework dylib"
    elif [[ -f "$x86_path" ]]; then
        mkdir -p "$(dirname "$staged_path")"
        cp "$x86_path" "$staged_path"
        chmod 755 "$staged_path"
        verify_architecture "$staged_path" "x86_64" "x86_64-only framework dylib"
    else
        die "framework dylib disappeared during merge: $relative"
    fi
}

build_universal_app() {
    local relative
    local inventory

    MERGE_TMP="$WORK_DIR/merge"
    mkdir -p "$WORK_DIR/app" "$MERGE_TMP"
    ditto "$ARM_APP" "$WORK_DIR/app/$APP_NAME"

    merge_macho "Contents/MacOS/$(basename "$ARM_EXECUTABLE")"
    inventory="$(framework_inventory_union "$ARM_APP" "$X86_APP")"
    while IFS= read -r relative; do
        [[ -n "$relative" ]] || continue
        merge_framework "$relative"
    done <<< "$inventory"

    # The arm64 code signature no longer describes the merged Mach-O files.
    rm -rf "$WORK_DIR/app/$APP_NAME/Contents/_CodeSignature"
    codesign --force --deep --sign - "$WORK_DIR/app/$APP_NAME" >/dev/null || die \
        "ad-hoc signing failed: $WORK_DIR/app/$APP_NAME"
    codesign --verify --deep --strict "$WORK_DIR/app/$APP_NAME" >/dev/null || die \
        "ad-hoc signature verification failed: $WORK_DIR/app/$APP_NAME"
}

build_archives() {
    local dmg_stage

    ditto -c -k --keepParent "$WORK_DIR/app/$APP_NAME" \
        "$WORK_DIR/$UNIVERSAL_ZIP" || die "failed to create Universal zip"

    if command -v hdiutil >/dev/null 2>&1; then
        dmg_stage="$WORK_DIR/dmg"
        mkdir -p "$dmg_stage"
        ditto "$WORK_DIR/app/$APP_NAME" "$dmg_stage/$APP_NAME"
        hdiutil create \
            -ov \
            -format UDZO \
            -volname "MacPlayer" \
            -srcfolder "$dmg_stage" \
            "$WORK_DIR/$UNIVERSAL_DMG" >/dev/null || die "failed to create Universal DMG"
    else
        printf 'package-macos-universal: hdiutil not found; skipped DMG creation\n' >&2
    fi
}

publish_outputs() {
    local source
    local destination

    PUBLISHING=1
    for source in \
        "$WORK_DIR/app/$APP_NAME" \
        "$WORK_DIR/$UNIVERSAL_ZIP"; do
        destination="$OUTPUT_DIR/$(basename "$source")"
        mv "$source" "$destination"
        PUBLISHED_PATHS+=("$destination")
    done

    if [[ -f "$WORK_DIR/$UNIVERSAL_DMG" ]]; then
        destination="$OUTPUT_DIR/$UNIVERSAL_DMG"
        mv "$WORK_DIR/$UNIVERSAL_DMG" "$destination"
        PUBLISHED_PATHS+=("$destination")
    fi

    PUBLISHING=0
    printf 'package-macos-universal: app  %s/%s\n' "$OUTPUT_DIR" "$APP_NAME"
    printf 'package-macos-universal: zip  %s/%s\n' "$OUTPUT_DIR" "$UNIVERSAL_ZIP"
    if [[ -f "$OUTPUT_DIR/$UNIVERSAL_DMG" ]]; then
        printf 'package-macos-universal: dmg  %s/%s\n' "$OUTPUT_DIR" "$UNIVERSAL_DMG"
    fi
}

main() {
    parse_arguments "$@"

    require_command uname
    [[ "$(uname -s)" == "Darwin" ]] || die "macOS packaging must run on macOS (Darwin)"
    require_command find
    require_command sort
    require_command awk
    require_command lipo
    require_command plutil
    require_command ditto
    require_command codesign
    require_command mktemp

    validate_inputs
    check_output_paths

    if (( CHECK_ONLY == 1 )); then
        printf 'package-macos-universal: inputs and output paths are valid\n'
        exit 0
    fi

    mkdir -p "$OUTPUT_DIR"
    WORK_DIR="$(mktemp -d "$OUTPUT_DIR/.macplayer-universal.XXXXXX")"
    build_universal_app
    build_archives
    publish_outputs
}

main "$@"
