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

# ── Resolve version from Cargo.toml ─────────────────────────────────
VERSION=$(grep '^version' "${PROJECT_ROOT}/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')
echo "=== Night Amplifier v${VERSION} distribution build ==="

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
fi

CPU_SUFFIX=""
if [[ "${TARGET_CPU}" == "cortex-a76" ]]; then
    CPU_SUFFIX="-pi5"
fi

ARTIFACT_NAME="night-amplifier-${VERSION}-${ARCH}${CPU_SUFFIX}${OS_SUFFIX}"
DIST_DIR="${PROJECT_ROOT}/dist/${ARTIFACT_NAME}"
BINARY_NAME="night_amplifier"

echo "CPU optimization: ${TARGET_CPU}"
echo "Use cross: ${USE_CROSS}"

# ── Step 1: Build web frontend ───────────────────────────────────────
if [[ "${BUILD_FRONTEND}" == "true" ]]; then
    echo ""
    echo "── Building web frontend ──"
    cd "${PROJECT_ROOT}/web"

    # Source nvm if available (CI and some dev environments)
    if [[ -f "$HOME/.nvm/nvm.sh" ]]; then
        # shellcheck disable=SC1091
        . "$HOME/.nvm/nvm.sh" 2>/dev/null || true
    fi

    npm ci --prefer-offline 2>/dev/null || npm install
    npm run build
    echo "Frontend built: web/dist/"
else
    echo ""
    echo "── Skipping frontend build (--no-frontend) ──"
    if [[ ! -f "${PROJECT_ROOT}/web/dist/index.html" ]]; then
        echo "WARNING: web/dist/ does not exist. Embedded assets will be empty." >&2
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

cp "${BINARY_PATH}" "${DIST_DIR}/night-amplifier"
cp "${PROJECT_ROOT}/LICENSE" "${DIST_DIR}/"
cp "${PROJECT_ROOT}/README.md" "${DIST_DIR}/"

# Create tarball
cd "${PROJECT_ROOT}/dist"
ARCHIVE="${ARTIFACT_NAME}.tar.gz"
tar czf "${ARCHIVE}" "${ARTIFACT_NAME}/"

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
        --binary "${DIST_DIR}/night-amplifier" \
        --version "${VERSION}" \
        --arch "${TARGET}" \
        --cpu-suffix "${CPU_SUFFIX}"
fi

echo ""
echo "=== Build complete ==="
