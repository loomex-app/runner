#!/bin/sh
set -eu

main() {

# This token is replaced with the immutable release version during assembly.
version="@LOOMEX_RELEASE_VERSION@"
repository="loomex-app/runner"
workflow="codex-plugin-release.yml"
issuer="https://token.actions.githubusercontent.com"
base="https://github.com/$repository/releases/download/v$version"

cosign_version="3.1.2"
sigstore_root_commit="a394944ec0ec1dd5e8ba50471e9ded37d88b5daa"
sigstore_root_sha256="6494e21ea73fa7ee769f85f57d5a3e6a08725eae1e38c755fc3517c9e6bc0b66"
sigstore_root_url="https://raw.githubusercontent.com/sigstore/root-signing/$sigstore_root_commit/targets/trusted_root.json"

fail() {
  echo "loomex installer: $*" >&2
  exit 1
}

for dependency in curl codex git python3 uname mktemp chmod awk dirname rm; do
  command -v "$dependency" >/dev/null 2>&1 || fail "required command not found: $dependency"
done
python3 - "$version" <<'PY' || fail "release asset contains an invalid embedded version"
import re
import sys

if not re.fullmatch(
    r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?",
    sys.argv[1],
):
    raise SystemExit(1)
PY

os=$(uname -s)
arch=$(uname -m)
case "$os" in
  Darwin) cosign_os=darwin ;;
  Linux) cosign_os=linux ;;
  *) fail "unsupported operating system: $os" ;;
esac
case "$arch" in
  arm64|aarch64) cosign_arch=arm64 ;;
  x86_64|amd64) cosign_arch=amd64 ;;
  *) fail "unsupported CPU architecture: $arch" ;;
esac

case "$cosign_os-$cosign_arch" in
  darwin-amd64) cosign_sha256="acd180f8b015be25240ca33abee8a1e564eb65cdf1a3cee4725456d2dceb7da6" ;;
  darwin-arm64) cosign_sha256="dec1c3f802320b19c2fbcf2dc7bcfb3f258e1c181a046c23a1a074bdf932f10a" ;;
  linux-amd64) cosign_sha256="f7622ed3cf22e55e1ae6377c080979ff77a22da9981c11df222a2e444991e7cf" ;;
  linux-arm64) cosign_sha256="90e7ae0b5dfd60f20816b52c012addf7fc055ebcc7bea4ce81c428ca8518c302" ;;
  *) fail "unsupported platform" ;;
esac

umask 077
temporary=$(mktemp -d "${TMPDIR:-/tmp}/loomex-codex-install.XXXXXX") || fail "could not create a temporary directory"
case "$temporary" in
  "${TMPDIR:-/tmp}"/loomex-codex-install.*) ;;
  *) fail "temporary directory has an unexpected path" ;;
esac
cleanup() {
  chmod -R u+w "$temporary" 2>/dev/null || true
  rm -rf -- "$temporary"
}
trap cleanup 0
trap 'exit 1' HUP INT TERM

download() {
  url=$1
  destination=$2
  curl --fail --location --silent --show-error \
    --proto '=https' --tlsv1.2 --retry 3 --retry-all-errors \
    --output "$destination" "$url"
  test -s "$destination" && test ! -L "$destination" || fail "downloaded file is missing or unsafe: $url"
}

sha256_file() {
  file=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file" | awk '{print $1}'
  else
    fail "sha256sum or shasum is required"
  fi
}

cosign_bin="$temporary/cosign"
cosign_name="cosign-$cosign_os-$cosign_arch"
download "https://github.com/sigstore/cosign/releases/download/v$cosign_version/$cosign_name" "$cosign_bin"
test "$(sha256_file "$cosign_bin")" = "$cosign_sha256" || fail "downloaded Cosign checksum did not match the pinned official release"
chmod 700 "$cosign_bin"

trusted_root="$temporary/trusted_root.json"
download "$sigstore_root_url" "$trusted_root"
test "$(sha256_file "$trusted_root")" = "$sigstore_root_sha256" || fail "Sigstore trusted-root checksum did not match the pinned official snapshot"
chmod 400 "$trusted_root"

installer="loomex-install-marketplace-$version.sh"
provenance="loomex-codex-marketplace-$version.provenance.json"
for name in "$installer" "$installer.sigstore.json" "$provenance" "$provenance.sigstore.json"; do
  download "$base/$name" "$temporary/$name"
done
chmod 500 "$temporary/$installer"
chmod 400 "$temporary/$installer.sigstore.json" "$temporary/$provenance" "$temporary/$provenance.sigstore.json"

identity="https://github.com/$repository/.github/workflows/$workflow@refs/tags/v$version"
"$cosign_bin" verify-blob \
  --bundle "$temporary/$installer.sigstore.json" \
  --trusted-root "$trusted_root" \
  --certificate-identity "$identity" \
  --certificate-oidc-issuer "$issuer" \
  "$temporary/$installer" >/dev/null
"$cosign_bin" verify-blob \
  --bundle "$temporary/$provenance.sigstore.json" \
  --trusted-root "$trusted_root" \
  --certificate-identity "$identity" \
  --certificate-oidc-issuer "$issuer" \
  "$temporary/$provenance" >/dev/null

# The versioned installer performs the only Codex mutation. It snapshots the
# previous marketplace/plugin state and restores it if any install step fails.
(
  cd "$temporary"
  PATH="$(dirname "$cosign_bin"):$PATH" \
    LOOMEX_COSIGN_TRUSTED_ROOT="$trusted_root" \
    "./$installer" "$version"
)

echo "Loomex Codex plugin $version is installed and enabled. Restart Codex or open a new task, then ask for any Loomex workflow naturally; Codex will automatically guide any required Runner setup, authentication, and workspace binding."
}

# Keep this invocation as the final bytes of the file. When used through
# `curl ... | sh`, the shell must receive and parse the complete function body
# before any download or Codex mutation can begin.
main "$@"
