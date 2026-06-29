#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAURI_DIR="${ROOT_DIR}/crates/loomex-tauri"
PROFILE="${LOOMEX_TAURI_PROFILE:-release}"
APP_NAME="Loomex"
APP_EXECUTABLE="loomex-tauri"
PACKAGE_DIR="${LOOMEX_TAURI_PACKAGE_DIR:-${ROOT_DIR}/target/loomex-tauri-package/${PROFILE}}"
INSTALL_DIR="${LOOMEX_TAURI_INSTALL_DIR:-${PACKAGE_DIR}/install/Applications}"
SIGN_IDENTITY="${LOOMEX_TAURI_SIGN_IDENTITY:--}"
LAUNCH_SMOKE="${LOOMEX_TAURI_SMOKE_LAUNCH:-0}"
SKIP_DMG="${LOOMEX_TAURI_SKIP_DMG:-0}"

usage() {
  cat <<'USAGE'
Usage:
  scripts/mac_packaging_smoke.sh

Builds a local Loomex.app package from the shared Tauri binary, signs it for
internal dogfooding, creates a DMG when hdiutil is available, and writes SHA-256
checksums.

Environment:
  LOOMEX_TAURI_PROFILE=release|debug       Cargo profile to build. Default: release.
  LOOMEX_TAURI_PACKAGE_DIR=/path/out       Output directory.
  LOOMEX_TAURI_INSTALL_DIR=/path/Apps      Temporary install directory for smoke.
  LOOMEX_TAURI_SIGN_IDENTITY="Developer ID Application: ..."
                                          Signing identity. Default "-" is ad-hoc.
  LOOMEX_TAURI_SMOKE_LAUNCH=1             Copy app to install dir, launch, quit,
                                          and launch again to verify restart.
  LOOMEX_TAURI_SKIP_DMG=1                 Skip DMG creation.

Distribution notarization is intentionally not performed here. See
docs/mac-packaging-signing-smoke.md for the Developer ID/notarytool flow.
USAGE
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "mac packaging smoke requires macOS" >&2
  exit 2
fi

for required in cargo codesign plutil shasum; do
  if ! command -v "${required}" >/dev/null 2>&1; then
    echo "missing required tool: ${required}" >&2
    exit 2
  fi
done

if ! command -v python3 >/dev/null 2>&1; then
  echo "missing required tool: python3" >&2
  exit 2
fi

if [[ "${PROFILE}" == "release" ]]; then
  CARGO_RELEASE_FLAG="--release"
  CARGO_BIN_DIR="${ROOT_DIR}/target/release"
elif [[ "${PROFILE}" == "debug" ]]; then
  CARGO_RELEASE_FLAG=""
  CARGO_BIN_DIR="${ROOT_DIR}/target/debug"
else
  echo "LOOMEX_TAURI_PROFILE must be release or debug" >&2
  exit 2
fi

CONFIG_JSON="${TAURI_DIR}/tauri.conf.json"
CONFIG_VALUES=$(python3 - "${CONFIG_JSON}" <<'PY'
import json
import sys
from pathlib import Path

config = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
print(config["identifier"])
print(config["version"])
print(config["productName"])
targets = set(config.get("bundle", {}).get("targets", []))
if "app" not in targets or "dmg" not in targets:
    raise SystemExit("tauri.conf.json must declare app and dmg bundle targets")
PY
)
IDENTIFIER="$(printf '%s\n' "${CONFIG_VALUES}" | sed -n '1p')"
VERSION="$(printf '%s\n' "${CONFIG_VALUES}" | sed -n '2p')"
PRODUCT_NAME="$(printf '%s\n' "${CONFIG_VALUES}" | sed -n '3p')"

echo "== build ${PRODUCT_NAME} Tauri binary =="
if [[ -n "${CARGO_RELEASE_FLAG}" ]]; then
  (cd "${ROOT_DIR}" && cargo build -p loomex-tauri --bin "${APP_EXECUTABLE}" "${CARGO_RELEASE_FLAG}")
else
  (cd "${ROOT_DIR}" && cargo build -p loomex-tauri --bin "${APP_EXECUTABLE}")
fi

BINARY="${CARGO_BIN_DIR}/${APP_EXECUTABLE}"
if [[ ! -x "${BINARY}" ]]; then
  echo "expected built binary at ${BINARY}" >&2
  exit 1
fi

APP_BUNDLE="${PACKAGE_DIR}/${APP_NAME}.app"
CONTENTS_DIR="${APP_BUNDLE}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"
MANIFEST="${PACKAGE_DIR}/packaging-smoke.json"
CHECKSUM_FILE="${PACKAGE_DIR}/SHA256SUMS"

rm -rf "${APP_BUNDLE}" "${PACKAGE_DIR}/${APP_NAME}.dmg" "${PACKAGE_DIR}/${APP_NAME}.app.tar.gz"
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}" "${INSTALL_DIR}"
cp "${BINARY}" "${MACOS_DIR}/${APP_EXECUTABLE}"
chmod 755 "${MACOS_DIR}/${APP_EXECUTABLE}"
cp "${TAURI_DIR}/icons/icon.png" "${RESOURCES_DIR}/icon.png"

cat >"${CONTENTS_DIR}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>${PRODUCT_NAME} Runner</string>
  <key>CFBundleExecutable</key>
  <string>${APP_EXECUTABLE}</string>
  <key>CFBundleIdentifier</key>
  <string>${IDENTIFIER}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>${PRODUCT_NAME}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${VERSION}</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

plutil -lint "${CONTENTS_DIR}/Info.plist" >/dev/null

echo "== sign app bundle =="
codesign --force --deep --sign "${SIGN_IDENTITY}" --timestamp=none "${APP_BUNDLE}"
codesign --verify --deep --strict --verbose=2 "${APP_BUNDLE}"

echo "== package app archive =="
tar -C "${PACKAGE_DIR}" -czf "${PACKAGE_DIR}/${APP_NAME}.app.tar.gz" "${APP_NAME}.app"

DMG_PATH=""
if [[ "${SKIP_DMG}" != "1" && "$(command -v hdiutil || true)" != "" ]]; then
  echo "== create dmg =="
  hdiutil create \
    -volname "${APP_NAME}" \
    -srcfolder "${APP_BUNDLE}" \
    -ov \
    -format UDZO \
    "${PACKAGE_DIR}/${APP_NAME}.dmg" >/dev/null
  DMG_PATH="${PACKAGE_DIR}/${APP_NAME}.dmg"
fi

echo "== write checksums =="
rm -f "${CHECKSUM_FILE}"
(
  cd "${PACKAGE_DIR}"
  shasum -a 256 "${APP_NAME}.app.tar.gz"
  if [[ -n "${DMG_PATH}" ]]; then
    shasum -a 256 "${APP_NAME}.dmg"
  fi
) | tee "${CHECKSUM_FILE}" >/dev/null
(cd "${PACKAGE_DIR}" && shasum -a 256 -c "${CHECKSUM_FILE}" >/dev/null)

LAUNCH_RESULT="skipped"
if [[ "${LAUNCH_SMOKE}" == "1" ]]; then
  if ! command -v open >/dev/null 2>&1; then
    echo "open is required for launch smoke" >&2
    exit 2
  fi
  echo "== install and launch smoke =="
  rm -rf "${INSTALL_DIR}/${APP_NAME}.app"
  cp -R "${APP_BUNDLE}" "${INSTALL_DIR}/${APP_NAME}.app"
  INSTALLED_APP="${INSTALL_DIR}/${APP_NAME}.app"
  open -n "${INSTALLED_APP}"
  sleep 3
  if ! pgrep -f "${INSTALLED_APP}/Contents/MacOS/${APP_EXECUTABLE}" >/dev/null; then
    echo "app did not launch from ${INSTALLED_APP}" >&2
    exit 1
  fi
  pkill -f "${INSTALLED_APP}/Contents/MacOS/${APP_EXECUTABLE}" || true
  sleep 1
  open -n "${INSTALLED_APP}"
  sleep 3
  if ! pgrep -f "${INSTALLED_APP}/Contents/MacOS/${APP_EXECUTABLE}" >/dev/null; then
    echo "app did not survive restart smoke from ${INSTALLED_APP}" >&2
    exit 1
  fi
  pkill -f "${INSTALLED_APP}/Contents/MacOS/${APP_EXECUTABLE}" || true
  LAUNCH_RESULT="passed"
fi

SPCTL_RESULT="skipped"
if command -v spctl >/dev/null 2>&1; then
  if spctl --assess --type execute --verbose "${APP_BUNDLE}" >/dev/null 2>&1; then
    SPCTL_RESULT="passed"
  else
    SPCTL_RESULT="not_accepted_for_gatekeeper_without_developer_id_notarization"
  fi
fi

python3 - "${MANIFEST}" "${APP_BUNDLE}" "${DMG_PATH}" "${CHECKSUM_FILE}" "${IDENTIFIER}" "${VERSION}" "${SIGN_IDENTITY}" "${LAUNCH_RESULT}" "${SPCTL_RESULT}" <<'PY'
import json
import sys
from pathlib import Path

manifest = {
    "schemaVersion": "loomex.tauri.macPackagingSmoke/v1",
    "appBundle": sys.argv[2],
    "dmg": sys.argv[3] or None,
    "checksums": sys.argv[4],
    "bundleIdentifier": sys.argv[5],
    "version": sys.argv[6],
    "signingIdentity": sys.argv[7],
    "launchSmoke": sys.argv[8],
    "gatekeeperAssessment": sys.argv[9],
    "secureStorage": {
        "backend": "system_keychain_when_available",
        "fallback": "local_file_fallback_with_warning",
    },
    "browserLogin": "uses_system_browser_url_from_login_device_start",
    "workspacePicker": "uses_tauri_native_dialog_command",
    "autoUpdate": "signed_manifest_helpers_ready_install_loop_external",
    "notarization": "required_for_public_distribution",
}
Path(sys.argv[1]).write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

echo "mac packaging smoke complete"
echo "app: ${APP_BUNDLE}"
if [[ -n "${DMG_PATH}" ]]; then
  echo "dmg: ${DMG_PATH}"
fi
echo "checksums: ${CHECKSUM_FILE}"
echo "manifest: ${MANIFEST}"
