#!/usr/bin/env bash
# Build a self-contained distribution archive for Night Amplifier.
#
# The archive contains a single binary with the Vue 3 web UI embedded,
# plus LICENSE and README.md.
#
# Usage:
#   ./scripts/build-dist.sh                                              # Native-optimized local build
#   ./scripts/build-dist.sh --target aarch64-unknown-linux-gnu           # Cross-compile for ARM64
#   ./scripts/build-dist.sh --target-cpu cortex-a76                      # Specific CPU optimization
#   ./scripts/build-dist.sh --cross --target aarch64-unknown-linux-gnu   # Use 'cross' tool
#   ./scripts/build-dist.sh --appimage                                   # Also create AppImage
#   ./scripts/build-dist.sh --no-frontend                                # Skip frontend build
#
# Environment variables:
#   CROSS=1               Same as --cross flag
#   EXTRA_CARGO_FLAGS     Additional flags passed to cargo/cross build

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# ── Defaults ─────────────────────────────────────────────────────────
TARGET=""
TARGET_CPU="native"
USE_CROSS="${CROSS:-0}"
BUILD_FRONTEND=true
BUILD_APPIMAGE=false
EXTRA_FEATURES="bundled-cfitsio"

# Auto-detect if we are on Windows (even before parsing args for default feature selection)
HOST_OS=$(rustc -vV | grep '^host:' | awk '{print $2}')
if [[ "${HOST_OS}" == *"-windows"* ]]; then
    # On Windows, bundled-cfitsio uses autotools which fails with MSVC.
    # We default to system/vcpkg cfitsio instead.
    EXTRA_FEATURES=""
fi

# ── Parse arguments ──────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)       TARGET="$2"; shift 2 ;;
        --target-cpu)   TARGET_CPU="$2"; shift 2 ;;
        --cross)        USE_CROSS=1; shift ;;
        --no-frontend)  BUILD_FRONTEND=false; shift ;;
        --appimage)     BUILD_APPIMAGE=true; shift ;;
        --features)     EXTRA_FEATURES="$2"; shift 2 ;;
        --help|-h)
            head -16 "$0" | tail -14
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

# ── Resolve version and package from Cargo.toml ──────────────────────
VERSION=$(grep '^version' "${PROJECT_ROOT}/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
PACKAGE_NAME=$(grep '^name' "${PROJECT_ROOT}/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')

DISPLAY_NAME="Night Amplifier"
if [[ "${PACKAGE_NAME}" == *"_pro"* ]]; then
    DISPLAY_NAME="Night Amplifier Pro"
fi

echo "=== ${DISPLAY_NAME} v${VERSION} distribution build ==="

# ── Resolve target triple ────────────────────────────────────────────
if [[ -z "${TARGET}" ]]; then
    TARGET=$(rustc -vV | grep '^host:' | awk '{print $2}')
    echo "Target (auto-detected): ${TARGET}"
else
    echo "Target: ${TARGET}"
fi

ARCH=$(echo "${TARGET}" | cut -d'-' -f1)

OS_SUFFIX=""
if [[ "${TARGET}" == *"-linux"* ]]; then
    OS_SUFFIX="-linux"
elif [[ "${TARGET}" == *"-windows"* ]]; then
    OS_SUFFIX="-windows"
elif [[ "${TARGET}" == *"-apple-darwin"* ]]; then
    OS_SUFFIX="-macos"
fi

CPU_SUFFIX=""
if [[ "${TARGET_CPU}" == "cortex-a76" ]]; then
    CPU_SUFFIX="-pi5"
fi

BINARY_NAME="${PACKAGE_NAME}"
OUT_BINARY_NAME=$(echo "${PACKAGE_NAME}" | tr '_' '-')
if [[ "${TARGET}" == *"-windows"* ]]; then
    BINARY_NAME="${BINARY_NAME}.exe"
    OUT_BINARY_NAME="${OUT_BINARY_NAME}.exe"
fi

# For Pro version, we want the suffix at the end of the artifact name
BASE_NAME=$(echo "${OUT_BINARY_NAME}" | sed 's/\.exe$//')
PRO_SUFFIX=""
if [[ "${BASE_NAME}" == *"-pro" ]]; then
    BASE_NAME=$(echo "${BASE_NAME}" | sed 's/-pro$//')
    PRO_SUFFIX="-pro"
fi

ARTIFACT_NAME="${BASE_NAME}-${VERSION}-${ARCH}${CPU_SUFFIX}${OS_SUFFIX}${PRO_SUFFIX}"
DIST_DIR="${PROJECT_ROOT}/dist/${ARTIFACT_NAME}"

echo "CPU optimization: ${TARGET_CPU}"
echo "Use cross: ${USE_CROSS}"

# ── Step 1: Build web frontend ───────────────────────────────────────
FRONTEND_DIR="${PROJECT_ROOT}/web"
if [[ ! -d "${FRONTEND_DIR}" ]]; then
    # Fallback to sibling night-amplifier/web (useful for Pro build)
    FRONTEND_DIR="${PROJECT_ROOT}/../night-amplifier/web"
fi

if [[ "${BUILD_FRONTEND}" == "true" ]]; then
    echo ""
    echo "── Building web frontend ──"
    if [[ ! -d "${FRONTEND_DIR}" ]]; then
        echo "Error: Frontend directory not found at ${FRONTEND_DIR}" >&2
        exit 1
    fi
    cd "${FRONTEND_DIR}"

    # Source nvm if available (CI and some dev environments)
    if [[ -f "$HOME/.nvm/nvm.sh" ]]; then
        # shellcheck disable=SC1091
        . "$HOME/.nvm/nvm.sh" 2>/dev/null || true
    fi

    npm ci --prefer-offline 2>/dev/null || npm install
    npm run build
    echo "Frontend built: ${FRONTEND_DIR}/dist/"
else
    echo ""
    echo "── Skipping frontend build (--no-frontend) ──"
    if [[ ! -f "${FRONTEND_DIR}/dist/index.html" ]]; then
        echo "WARNING: ${FRONTEND_DIR}/dist/ does not exist. Embedded assets will be empty." >&2
    fi
fi

# ── Step 2: Build Rust binary ────────────────────────────────────────
echo ""
echo "── Building Rust binary (release) ──"
cd "${PROJECT_ROOT}"

# WORKAROUND: fitsio-sys pollutes the global cargo registry with compiled object files.
# If we switch between native and cross-compilation, the linker will crash trying to link
# an x86_64 libcfitsio.a into an aarch64 binary. We must clean the cached objects.
if [[ -d "${HOME}/.cargo/registry/src" ]]; then
    find "${HOME}/.cargo/registry/src" -type f -name "*.o" -path "*/fitsio-sys-*/ext/cfitsio/*" -delete 2>/dev/null || true
    find "${HOME}/.cargo/registry/src" -type f -name "*.a" -path "*/fitsio-sys-*/ext/cfitsio/lib/*" -delete 2>/dev/null || true
fi

export RUSTFLAGS="-C target-cpu=${TARGET_CPU} ${RUSTFLAGS:-}"
echo "RUSTFLAGS=${RUSTFLAGS}"

BUILD_CMD="cargo"
if [[ "${USE_CROSS}" == "1" ]]; then
    if ! command -v cross &>/dev/null; then
        echo "Error: 'cross' is not installed. Install with: cargo install cross" >&2
        exit 1
    fi
    BUILD_CMD="cross"
fi

BUILD_ARGS=(
    build
    --release
    --no-default-features
    --features "${EXTRA_FEATURES}"
    --target "${TARGET}"
)

# Append any extra cargo flags
if [[ -n "${EXTRA_CARGO_FLAGS:-}" ]]; then
    # shellcheck disable=SC2206
    BUILD_ARGS+=(${EXTRA_CARGO_FLAGS})
fi

echo "${BUILD_CMD} ${BUILD_ARGS[*]}"
${BUILD_CMD} "${BUILD_ARGS[@]}"

# ── Step 3: Package distribution ─────────────────────────────────────
echo ""
echo "── Packaging distribution ──"

BINARY_PATH="${PROJECT_ROOT}/target/${TARGET}/release/${BINARY_NAME}"
if [[ ! -f "${BINARY_PATH}" ]]; then
    echo "Error: Binary not found at ${BINARY_PATH}" >&2
    exit 1
fi

rm -rf "${DIST_DIR}"
mkdir -p "${DIST_DIR}"

cp "${BINARY_PATH}" "${DIST_DIR}/${OUT_BINARY_NAME}"
cp "${PROJECT_ROOT}/LICENSE" "${DIST_DIR}/"
cp "${PROJECT_ROOT}/README.md" "${DIST_DIR}/"

# Create archive
cd "${PROJECT_ROOT}/dist"
if [[ "${TARGET}" == *"-windows"* ]]; then
    ARCHIVE="${ARTIFACT_NAME}.zip"
    if command -v zip >/dev/null 2>&1; then
        zip -r "${ARCHIVE}" "${ARTIFACT_NAME}/"
    elif command -v 7z >/dev/null 2>&1; then
        7z a -tzip "${ARCHIVE}" "${ARTIFACT_NAME}/"
    else
        echo "Error: Neither 'zip' nor '7z' found for packaging."
        exit 1
    fi
else
    ARCHIVE="${ARTIFACT_NAME}.tar.gz"
    tar czf "${ARCHIVE}" "${ARTIFACT_NAME}/"
fi

ARCHIVE_PATH="${PROJECT_ROOT}/dist/${ARCHIVE}"
SIZE=$(du -h "${ARCHIVE_PATH}" | awk '{print $1}')
SHA256=$(sha256sum "${ARCHIVE_PATH}" | awk '{print $1}')

echo ""
echo "── Distribution archive created ──"
echo "  File:   dist/${ARCHIVE}"
echo "  Size:   ${SIZE}"
echo "  SHA256: ${SHA256}"

# ── Step 4: Optional AppImage ────────────────────────────────────────
if [[ "${BUILD_APPIMAGE}" == "true" ]]; then
    echo ""
    echo "── Building AppImage ──"
    "${SCRIPT_DIR}/build-appimage.sh" \
        --binary "${DIST_DIR}/${OUT_BINARY_NAME}" \
        --version "${VERSION}" \
        --arch "${TARGET}" \
        --cpu-suffix "${CPU_SUFFIX}" \
        --app-name "${DISPLAY_NAME}" \
        --app-id "$(echo "${OUT_BINARY_NAME}" | sed 's/\.exe$//')" \
        --icon "${FRONTEND_DIR}/src/assets/night_amplifier_logo_256.png"
fi

echo ""
echo "=== Build complete ==="
