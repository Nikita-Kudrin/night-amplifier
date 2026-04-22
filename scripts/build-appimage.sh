#!/usr/bin/env bash
# Build an AppImage from an already-compiled Night Amplifier binary.
#
# Called by build-dist.sh --appimage or standalone:
#   ./scripts/build-appimage.sh --binary dist/night-amplifier --version 0.1.0 --arch x86_64-unknown-linux-gnu
#
# Requires: appimagetool (downloaded automatically if not on PATH)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

BINARY=""
VERSION="0.0.0"
ARCH_TRIPLE=""
CPU_SUFFIX=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary)     BINARY="$2"; shift 2 ;;
        --version)    VERSION="$2"; shift 2 ;;
        --arch)       ARCH_TRIPLE="$2"; shift 2 ;;
        --cpu-suffix) CPU_SUFFIX="$2"; shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "${BINARY}" || ! -f "${BINARY}" ]]; then
    echo "Error: --binary must point to an existing file" >&2
    exit 1
fi

# Map Rust target triple to AppImage arch label
case "${ARCH_TRIPLE}" in
    x86_64*)  ARCH="x86_64" ;;
    aarch64*) ARCH="aarch64" ;;
    *)        ARCH="$(uname -m)" ;;
esac

APPDIR="${PROJECT_ROOT}/dist/NightAmplifier.AppDir"
APPIMAGE_NAME="Night_Amplifier-${VERSION}-${ARCH}${CPU_SUFFIX}.AppImage"

rm -rf "${APPDIR}"
mkdir -p "${APPDIR}/usr/bin"

# ── Binary ───────────────────────────────────────────────────────────
cp "${BINARY}" "${APPDIR}/usr/bin/night-amplifier"
chmod +x "${APPDIR}/usr/bin/night-amplifier"

# ── Desktop entry ────────────────────────────────────────────────────
cat > "${APPDIR}/night-amplifier.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=Night Amplifier
Comment=EAA Live Stacking Server for Astronomy
Exec=night-amplifier
Icon=night-amplifier
Categories=Science;Astronomy;
Terminal=true
EOF

# ── Icon ─────────────────────────────────────────────────────────────
LOGO_SRC="${PROJECT_ROOT}/web/src/assets/night_amplifier_logo_256.png"
if [[ -f "${LOGO_SRC}" ]]; then
    cp "${LOGO_SRC}" "${APPDIR}/night-amplifier.png"
else
    echo "WARNING: Logo not found at ${LOGO_SRC}. AppImage will have no icon." >&2
fi

# ── AppRun launcher ─────────────────────────────────────────────────
cat > "${APPDIR}/AppRun" <<'RUNEOF'
#!/bin/bash
SELF="$(readlink -f "$0")"
APPDIR="$(dirname "${SELF}")"
exec "${APPDIR}/usr/bin/night-amplifier" "$@"
RUNEOF
chmod +x "${APPDIR}/AppRun"

# ── Build AppImage ───────────────────────────────────────────────────
APPIMAGETOOL=""
if command -v appimagetool &>/dev/null; then
    APPIMAGETOOL="appimagetool"
else
    # Download appimagetool
    TOOL_PATH="${PROJECT_ROOT}/dist/appimagetool"
    if [[ ! -x "${TOOL_PATH}" ]]; then
        echo "Downloading appimagetool..."
        TOOL_ARCH="$(uname -m)"
        curl -fsSL -o "${TOOL_PATH}" \
            "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${TOOL_ARCH}.AppImage"
        chmod +x "${TOOL_PATH}"
    fi
    APPIMAGETOOL="${TOOL_PATH}"
fi

cd "${PROJECT_ROOT}/dist"
ARCH="${ARCH}" "${APPIMAGETOOL}" "${APPDIR}" "${APPIMAGE_NAME}" || {
    echo "WARNING: appimagetool failed. AppImage not created." >&2
    echo "  The tar.gz archive is still available." >&2
    exit 0
}

APPIMAGE_PATH="${PROJECT_ROOT}/dist/${APPIMAGE_NAME}"
if [[ -f "${APPIMAGE_PATH}" ]]; then
    SIZE=$(du -h "${APPIMAGE_PATH}" | awk '{print $1}')
    SHA256=$(sha256sum "${APPIMAGE_PATH}" | awk '{print $1}')
    echo ""
    echo "── AppImage created ──"
    echo "  File:   dist/${APPIMAGE_NAME}"
    echo "  Size:   ${SIZE}"
    echo "  SHA256: ${SHA256}"
fi

# Cleanup
rm -rf "${APPDIR}"
