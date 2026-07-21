# Loomex for Codex

The Loomex plugin makes Codex a conversational client for Loomex. It can browse
workflows, bind an explicitly selected local workspace, start and follow durable
runs, and surface human requests and approval decisions. The Tauri application
remains supported; both clients use the same backend and per-user Runner service.

## User experience

Official macOS and Linux release packages include both the `loomex-mcp` adapter
and the matching Loomex Runner runtime. On first use, ask Codex to set up Loomex.
The plugin previews the per-user service changes, verifies and installs that
bundled runtime into Loomex's stable runtime directory after approval, starts
it, and guides authentication. There is no separate Runner download in the
normal installation flow.

## Install from the Loomex marketplace

Each release publishes a complete Codex marketplace snapshot, provenance, and
`loomex-install-marketplace-<version>.sh`, each with a keyless Sigstore bundle.
Download the installer and its `.sigstore.json` bundle together with
`loomex-codex-marketplace-<version>.provenance.json` and its bundle. From that
directory, run this single fail-closed bootstrap command. It opens the installer
twice before verification, confirms both descriptors identify the same file,
verifies one descriptor against the pinned GitHub issuer/workflow/tag identity,
and executes the other descriptor without reopening the pathname:

```bash
sh -eu -c '
version=$1
installer="loomex-install-marketplace-$version.sh"
test -f "$installer" && test ! -L "$installer"
test -f "$installer.sigstore.json" && test ! -L "$installer.sigstore.json"
exec 3< "$installer"
exec 4< "$installer"
test /dev/fd/3 -ef /dev/fd/4
cosign verify-blob \
  --bundle "$installer.sigstore.json" \
  --certificate-identity "https://github.com/loomex-app/runner/.github/workflows/codex-plugin-release.yml@refs/tags/v$version" \
  --certificate-oidc-issuer "https://token.actions.githubusercontent.com" \
  /dev/fd/3
exec sh /dev/fd/4 "$version"
' sh <release-version>
```

Only the verified exact 40-character commit value is used for installation.
The exact commit is content-addressed and cannot move. Do not substitute the
`codex-plugin-marketplace-v<version>` publication branch, a tag, or another
symbolic ref: those names can be moved. The first command is a one-time
marketplace registration for that exact snapshot. The second installs the
adapter, Runner, and `loomex-core`-based runtime together; users do not install
a separate native package. Start a new Codex task after installing or upgrading
so the new MCP tools are loaded.

For an offline/local install, first verify
`loomex-codex-marketplace-<version>.zip.sigstore.json` with the same pinned
issuer and workflow identity. Only then extract the ZIP, pass its directory to
`codex plugin marketplace add`, and run the same
`codex plugin add loomex@loomex` command.

Run the verified installer again to upgrade. It inspects the existing Codex
marketplace and plugin state, does nothing when the same exact commit is already
installed, and otherwise replaces the marketplace with the newly verified
exact commit. If any upgrade step fails, it restores the previous checkout by
its captured exact commit and restores whether the plugin was installed. A
pre-existing sparse or disabled Loomex installation is rejected before any
change because the current Codex CLI cannot reproduce that state safely.

The release job keyless-signs and verifies the marketplace ZIP and provenance
with Sigstore/Fulcio and records the bundles in Sigstore's transparency log,
recreates the exact Git tree, rejects any commit mismatch, and only then
publishes it. The signed provenance binds the release version, archive SHA-256,
Git tree, repository, and exact commit. The trust anchor is the pinned GitHub
OIDC issuer plus exact repository/workflow/tag identity shown above—not a public
key downloaded beside the release assets.

The offline/local path verifies the marketplace ZIP's `.sigstore.json` bundle
with the same issuer and workflow identity before installing the extracted
directory. Both paths avoid trusting a mutable branch or co-located replacement
key to select installation bytes.

Typical prompts:

- `Show my Loomex workflows.`
- `Bind this repository to Loomex and run release-review.`
- `Wait for run 123 and show me any human requests.`
- `Show pending Loomex approvals.`

Every tool publishes a tool-specific `outputSchema` and returns the same
discriminated structured envelope. Successful calls have
`{schemaVersion: "loomex.mcp/v1", ok: true, tool, data, meta}`; failed calls
have `{schemaVersion: "loomex.mcp/v1", ok: false, tool, error, meta}`. The
`data` schema is specific to the selected tool (runs, workflows, approvals,
logs, and so on), while entity objects allow additive backend fields so a
compatible server upgrade does not break Codex. The adapter validates runtime
responses before exposing them as `structuredContent`.

Setup changes are explicit. `loomex_setup_plan` returns the exact paths and
service action before `loomex_setup_apply` makes persistent per-user changes.
System-wide installation and administrator access are not required.

## Durable execution and the closed-Codex boundary

`loomex-mcp` is an adapter, not the execution engine. Before the service exists,
it routes setup, authentication, scope, binding, and service-control operations
through the bundled `loomex` bootstrap command. Workflow execution and live run
operations use the authenticated local-control socket of the durable service;
read-only status, diagnostics, and logs can fall back to the bootstrap command
when that socket is unavailable.

Long-running work is owned by the durable Loomex backend and the headless Runner
service. Closing or restarting Codex therefore does not cancel an accepted run.
Run state, human requests, approvals, and logs remain available after
reconnection.

Codex cannot display a new question while the Codex application is closed. A
pending request remains durable and appears when the plugin reconnects; the
Tauri app may also be used to answer it. The initial setup operation itself must
finish before Codex is closed so that the Runner service has been installed.

## Local workspace safety

Loomex operates only inside an explicit workspace binding. Binding creation
shows the canonical root and project before it is persisted. The Runner owns
path containment, symlink escape prevention, execution policy, cancellation,
and audit logging. Codex approval prompts complement, but do not replace, those
Runner controls.

## Source and release packages

This source tree intentionally contains no unsigned or placeholder executable.
CI builds each native `loomex-mcp` adapter and matching `loomex` runtime,
verifies both, writes their sizes and SHA-256 digests into
`packaging/runtime-manifest.json`, and then assembles the macOS/Linux plugin
archive. See [packaging/README.md](packaging/README.md).

To use a source checkout while developing, set `LOOMEX_MCP_BINARY` to an
absolute path to a locally built adapter and set
`LOOMEX_ALLOW_DEVELOPMENT_BINARY=1`. Keep the matching `loomex` executable next
to that adapter, or set `LOOMEX_RUNNER_BINARY` to its absolute path. Official
packages never need these variables.
The dependency-free `/bin/sh` launcher used by official macOS/Linux packages
ignores development overrides whenever an official runtime manifest is present.

## Development validation

From this plugin directory:

```bash
node --test tests/*.test.mjs
node scripts/validate-package.mjs
python3 /path/to/plugin-creator/scripts/validate_plugin.py .
python3 /path/to/skill-creator/scripts/quick_validate.py skills/loomex
```

Release assembly must additionally pass:

```bash
node scripts/validate-package.mjs --release
```

That mode requires a real native executable and a complete digest manifest.
