#!/bin/sh
set -eu

version=${1:-}
case "$version" in
  ""|*[!0-9A-Za-z.+-]*)
    echo "usage: $0 <release-version>" >&2
    exit 2
    ;;
esac

provenance="loomex-codex-marketplace-$version.provenance.json"
bundle="$provenance.sigstore.json"
test -f "$provenance" && test ! -L "$provenance"
test -f "$bundle" && test ! -L "$bundle"
command -v cosign >/dev/null
command -v python3 >/dev/null
command -v codex >/dev/null
command -v git >/dev/null

umask 077
temporary="$(mktemp -d "${TMPDIR:-/tmp}/loomex-marketplace-install.XXXXXX")"
cleanup() {
  chmod u+w "$temporary/provenance.json" "$temporary/provenance.sigstore.json" 2>/dev/null || true
  rm -f "$temporary/provenance.json" "$temporary/provenance.sigstore.json"
  rmdir "$temporary" 2>/dev/null || true
}
trap cleanup 0
trap 'exit 1' HUP INT TERM
cp "$provenance" "$temporary/provenance.json"
cp "$bundle" "$temporary/provenance.sigstore.json"
chmod 400 "$temporary/provenance.json" "$temporary/provenance.sigstore.json"

# Open the verified payload twice before verification. The descriptors refer to
# the same immutable copied inode but have independent offsets: Cosign consumes
# fd 3 and Python parses fd 4. Replacing the original pathname cannot change
# either stream after these opens.
exec 3< "$temporary/provenance.json"
exec 4< "$temporary/provenance.json"
test /dev/fd/3 -ef /dev/fd/4
cosign verify-blob \
  --bundle "$temporary/provenance.sigstore.json" \
  --certificate-identity "https://github.com/loomex-app/runner/.github/workflows/codex-plugin-release.yml@refs/tags/v$version" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  /dev/fd/3

python3 - "$version" 4<&4 <<'PY'
import json
import os
import re
import stat
import subprocess
import sys
from pathlib import Path


MARKETPLACE = "loomex"
PLUGIN_ID = "loomex@loomex"
REPOSITORY = "loomex-app/runner"
SHA1 = re.compile(r"[0-9a-f]{40}")


def fail(message):
    raise RuntimeError(message)


def command(arguments, *, json_output=False):
    result = subprocess.run(
        arguments,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip() or "no diagnostic output"
        fail(f"command failed ({result.returncode}): {' '.join(arguments)}: {detail}")
    if not json_output:
        return result.stdout.strip()
    try:
        value = json.loads(result.stdout)
    except json.JSONDecodeError as error:
        fail(f"command returned invalid JSON: {' '.join(arguments)}: {error}")
    if not isinstance(value, dict):
        fail(f"command returned a non-object JSON document: {' '.join(arguments)}")
    return value


def codex_json(*arguments):
    return command(["codex", *arguments, "--json"], json_output=True)


def trusted_repository_source(source):
    if not isinstance(source, str) or "\n" in source or "\r" in source:
        return False
    return bool(
        re.fullmatch(
            r"(?:loomex-app/runner|https://github\.com/loomex-app/runner(?:\.git)?|"
            r"ssh://git@github\.com/loomex-app/runner(?:\.git)?|"
            r"git@github\.com:loomex-app/runner(?:\.git)?)",
            source,
        )
    )


def git_config_bool(root, key):
    result = subprocess.run(
        ["git", "-C", root, "config", "--bool", key],
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode == 1 and not result.stdout.strip():
        return False
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip() or "no diagnostic output"
        fail(f"failed to inspect Git config {key}: {detail}")
    value = result.stdout.strip().lower()
    if value not in {"true", "false"}:
        fail(f"Git config {key} did not return a boolean")
    return value == "true"


def codex_config_path():
    configured_home = os.environ.get("CODEX_HOME")
    home = Path(configured_home) if configured_home else Path.home() / ".codex"
    return home / "config.toml"


def decode_toml_string(value, field):
    # Codex writes these fields as TOML basic strings. JSON has the same quoted
    # string escaping for every character Codex emits here, so this remains
    # dependency-free on systems whose Python predates tomllib.
    try:
        decoded = json.loads(value)
    except json.JSONDecodeError as error:
        fail(f"Codex config has an invalid {field}: {error}")
    if not isinstance(decoded, str):
        fail(f"Codex config has a non-string {field}")
    return decoded


def marketplace_config(source, commit):
    path = codex_config_path()
    try:
        mode = os.lstat(path).st_mode
    except FileNotFoundError:
        fail("Codex config is missing for the existing loomex marketplace")
    if not stat.S_ISREG(mode):
        fail("Codex config is not a regular file")
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as error:
        fail(f"failed to read Codex config: {error}")

    section = None
    values = {}
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("["):
            if stripped == "[marketplaces.loomex]":
                if section is not None:
                    fail("Codex config contains duplicate loomex marketplace sections")
                section = True
                continue
            if section is True:
                section = False
        if section is not True or not stripped or stripped.startswith("#"):
            continue
        match = re.fullmatch(r"([A-Za-z0-9_-]+)\s*=\s*(.*?)\s*", stripped)
        if match:
            key, value = match.groups()
            if key in values:
                fail(f"Codex config contains duplicate loomex marketplace field {key}")
            values[key] = value

    if section is None:
        fail("Codex config is missing the loomex marketplace section")
    for key in ("source_type", "source", "ref"):
        if key not in values:
            fail(f"Codex config is missing loomex marketplace field {key}")
    source_type = decode_toml_string(values["source_type"], "source_type")
    configured_source = decode_toml_string(values["source"], "source")
    configured_ref = decode_toml_string(values["ref"], "ref")
    if source_type != "git":
        fail("Codex config does not describe a Git loomex marketplace")
    if configured_source != source or not trusted_repository_source(configured_source):
        fail("Codex config and marketplace list disagree about the loomex source")
    sparse = values.get("sparse_paths")
    if sparse is not None:
        try:
            sparse = json.loads(sparse)
        except json.JSONDecodeError as error:
            fail(f"Codex config has invalid loomex sparse_paths: {error}")
        if sparse != []:
            fail("cannot safely preserve sparse loomex marketplace config")

    last_revision_value = values.get("last_revision")
    last_revision = (
        decode_toml_string(last_revision_value, "last_revision")
        if last_revision_value is not None
        else None
    )
    if (
        SHA1.fullmatch(configured_ref)
        and configured_ref == commit
        and last_revision == commit
    ):
        return commit
    return None


def install_metadata(root, source, commit):
    path = Path(root) / ".codex-marketplace-install.json"
    try:
        mode = os.lstat(path).st_mode
    except FileNotFoundError:
        return None
    if not stat.S_ISREG(mode):
        fail("loomex marketplace install metadata is not a regular file")
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        fail(f"failed to read loomex marketplace install metadata: {error}")
    if not isinstance(value, dict):
        fail("loomex marketplace install metadata is not a JSON object")
    if value.get("source_type") != "git":
        fail("loomex marketplace install metadata is not for Git")
    metadata_source = value.get("source")
    sparse_paths = value.get("sparse_paths")
    ref_name = value.get("ref_name")
    revision = value.get("revision")
    if metadata_source != source or not trusted_repository_source(metadata_source):
        fail("loomex marketplace list and install metadata disagree about the source")
    if sparse_paths != []:
        fail("cannot safely preserve sparse loomex marketplace metadata")
    if not isinstance(ref_name, str) or not isinstance(revision, str):
        return None
    if not SHA1.fullmatch(ref_name) or not SHA1.fullmatch(revision):
        return None
    if ref_name != commit or revision != commit:
        return None
    return commit


def sparse_checkout_file_present(root):
    relative = command(
        ["git", "-C", root, "rev-parse", "--git-path", "info/sparse-checkout"]
    )
    path = Path(relative)
    if not path.is_absolute():
        path = Path(root) / path
    try:
        mode = os.lstat(path).st_mode
    except FileNotFoundError:
        return False
    if not stat.S_ISREG(mode):
        fail("Git sparse-checkout state is not a regular file")
    try:
        return bool(path.read_bytes().strip())
    except OSError as error:
        fail(f"failed to inspect Git sparse-checkout state: {error}")


def marketplace_entry(*, read_metadata=True):
    document = codex_json("plugin", "marketplace", "list")
    entries = document.get("marketplaces")
    if not isinstance(entries, list):
        fail("Codex marketplace list JSON is missing marketplaces[]")
    matches = [entry for entry in entries if isinstance(entry, dict) and entry.get("name") == MARKETPLACE]
    if len(matches) > 1:
        fail("Codex reported duplicate loomex marketplace entries")
    if not matches:
        return None
    entry = matches[0]
    source = entry.get("marketplaceSource")
    root = entry.get("root")
    if not isinstance(source, dict) or source.get("sourceType") != "git":
        fail("existing loomex marketplace is not a Git marketplace")
    url = source.get("source")
    if not trusted_repository_source(url):
        fail("existing loomex marketplace points at an unexpected repository")
    if not isinstance(root, str) or not Path(root).is_absolute():
        fail("existing loomex marketplace has an invalid checkout root")
    if (
        git_config_bool(root, "core.sparseCheckout")
        or git_config_bool(root, "core.sparseCheckoutCone")
        or sparse_checkout_file_present(root)
    ):
        fail("cannot safely preserve a sparse loomex marketplace checkout")
    commit = command(["git", "-C", root, "rev-parse", "--verify", "HEAD^{commit}"])
    if not SHA1.fullmatch(commit):
        fail("existing loomex marketplace checkout is not at an exact SHA-1 commit")
    origin = command(["git", "-C", root, "remote", "get-url", "origin"])
    if origin != url or not trusted_repository_source(origin):
        fail("loomex marketplace list and Git origin disagree about the source")
    return {
        "source": url,
        "commit": commit,
        "configured_commit": (
            commit
            if read_metadata
            and marketplace_config(url, commit) == commit
            and install_metadata(root, url, commit) == commit
            else None
        ),
    }


def plugin_state():
    document = codex_json("plugin", "list", "--available")
    installed = document.get("installed")
    available = document.get("available")
    if not isinstance(installed, list) or not isinstance(available, list):
        fail("Codex plugin list JSON is missing installed[] or available[]")
    matches = []
    for entry in installed + available:
        if isinstance(entry, dict) and entry.get("pluginId") == PLUGIN_ID:
            matches.append(entry)
    if len(matches) > 1:
        fail("Codex reported duplicate loomex plugin entries")
    if not matches:
        return {"installed": False, "enabled": False}
    entry = matches[0]
    is_installed = entry.get("installed")
    is_enabled = entry.get("enabled")
    if not isinstance(is_installed, bool) or not isinstance(is_enabled, bool):
        fail("Codex reported an invalid loomex plugin state")
    if is_enabled and not is_installed:
        fail("Codex reported loomex enabled but not installed")
    return {"installed": is_installed, "enabled": is_enabled}


def snapshot():
    marketplace = marketplace_entry()
    plugin = plugin_state()
    if marketplace is None and plugin["installed"]:
        fail("loomex is installed without its marketplace")
    if plugin["installed"] and not plugin["enabled"]:
        fail("cannot safely preserve a disabled loomex installation with this Codex CLI")
    return {"marketplace": marketplace, "plugin": plugin}


def remove_current():
    current_plugin = plugin_state()
    if current_plugin["installed"]:
        codex_json("plugin", "remove", PLUGIN_ID)
    # A failed upgrade may have left malformed activation metadata. It is not
    # needed to safely identify and remove the trusted Git checkout.
    if marketplace_entry(read_metadata=False) is not None:
        codex_json("plugin", "marketplace", "remove", MARKETPLACE)


def add_marketplace(source, commit):
    if not SHA1.fullmatch(commit):
        fail("refusing to install a mutable or invalid marketplace ref")
    codex_json("plugin", "marketplace", "add", source, "--ref", commit)
    # Codex 0.144.6 records ref in config on add, then upgrade activates the
    # exact revision and writes .codex-marketplace-install.json. The list JSON
    # intentionally exposes neither ref nor sparse paths.
    codex_json("plugin", "marketplace", "upgrade", MARKETPLACE)


def verify_state(expected_marketplace, installed):
    current_marketplace = marketplace_entry()
    current_plugin = plugin_state()
    if expected_marketplace is None:
        if current_marketplace is not None:
            fail("marketplace remained installed after rollback")
    else:
        if current_marketplace is None:
            fail("marketplace is missing after transaction")
        if current_marketplace["commit"] != expected_marketplace["commit"]:
            fail("marketplace checkout does not match the expected exact commit")
        if current_marketplace["configured_commit"] != expected_marketplace["commit"]:
            fail("marketplace activation metadata does not prove the expected exact ref")
        # Codex normalizes owner/repo to an HTTPS .git URL. Both forms are the
        # same pinned repository; untrusted repository forms were rejected
        # while reading each state.
        if not trusted_repository_source(expected_marketplace["source"]):
            fail("expected marketplace source is not the trusted repository")
    if current_plugin["installed"] != installed:
        fail("loomex plugin installation state does not match the expected state")
    if installed and not current_plugin["enabled"]:
        fail("loomex plugin is installed but not enabled")


def restore(previous):
    remove_current()
    old_marketplace = previous["marketplace"]
    old_installed = previous["plugin"]["installed"]
    if old_marketplace is not None:
        # A prior branch or tag is intentionally not restored symbolically. The
        # checked-out commit captured before mutation is the only rollback ref.
        add_marketplace(old_marketplace["source"], old_marketplace["commit"])
        if old_installed:
            codex_json("plugin", "add", PLUGIN_ID)
    verify_state(old_marketplace, old_installed)


os.lseek(4, 0, os.SEEK_SET)
payload = json.loads(os.read(4, 16 * 1024 * 1024))
marketplace = payload["marketplace"]
marketplace_commit = marketplace["commit"]
if not (
    payload["schemaVersion"] == 1
    and payload["pluginVersion"] == sys.argv[1]
    and marketplace["repository"] == REPOSITORY
    and marketplace["gitObjectFormat"] == "sha1"
    and isinstance(marketplace_commit, str)
    and SHA1.fullmatch(marketplace_commit)
):
    fail("verified provenance does not describe the expected immutable marketplace")

previous = snapshot()
old_marketplace = previous["marketplace"]
old_plugin = previous["plugin"]

# Exact-ref, installed state is already the desired result. A matching checkout
# configured through a mutable branch/tag is deliberately rewritten below.
if (
    old_marketplace is not None
    and old_marketplace["commit"] == marketplace_commit
    and old_marketplace["configured_commit"] == marketplace_commit
    and old_plugin["installed"]
    and old_plugin["enabled"]
):
    sys.exit(0)

install_source = old_marketplace["source"] if old_marketplace is not None else REPOSITORY
try:
    if old_marketplace is not None:
        if old_plugin["installed"]:
            codex_json("plugin", "remove", PLUGIN_ID)
        codex_json("plugin", "marketplace", "remove", MARKETPLACE)
    add_marketplace(install_source, marketplace_commit)
    codex_json("plugin", "add", PLUGIN_ID)
    verify_state(
        {"source": install_source, "commit": marketplace_commit},
        True,
    )
except Exception as original_error:
    try:
        restore(previous)
    except Exception as rollback_error:
        fail(f"installation failed: {original_error}; rollback also failed: {rollback_error}")
    fail(f"installation failed and the prior state was restored: {original_error}")
PY
