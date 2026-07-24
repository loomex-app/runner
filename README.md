# loomex-runner

`loomex-runner` is the user-side process that connects a local directory to a Loomex workflow.
It does not run Codex by default. Loomex runs AI on the server side, then sends bounded jobs to
the runner to read files, write files, or execute approved commands inside the configured workspace.

This Python runner is the current spike and baseline compatibility reference. It is not the
production target for Loomex Runner; the production command contract is `loomex`.
See [docs/spike-baseline.md](docs/spike-baseline.md) for frozen spike evidence and repeatable
baseline smoke commands.
See [docs/legacy-compatibility.md](docs/legacy-compatibility.md) for the compatibility window,
REST long-poll removal target, command mapping, and config migration rules.

## Legacy warning

`loomex-runner` is legacy compatibility tooling. Every real command prints a deprecation warning
to stderr while keeping stdout machine-readable. Use the production `loomex` command path for new
work:

| Legacy | Production path |
| --- | --- |
| `loomex-runner login` | `loomex login` |
| `loomex-runner workspace add NAME PATH` | `loomex bind --workspace PATH` |
| `loomex-runner connect --workspace PATH` | `loomex bind --workspace PATH` + `loomex runner start` |
| `loomex-runner run --workflow ID` | `loomex workflow run ID` |
| `loomex-runner start --workspace NAME` | `loomex runner start` |
| `loomex-runner doctor` | `loomex runner doctor` |

## Local install

```bash
python3 -m pip install -e /Users/oveysrostami/Codes/Loomex/loomex-runner
```

## One-shot workflow execution

Current legacy spike path is:

```bash
loomex-runner login \
  --server http://127.0.0.1:28000/api/v1/runner-control \
  --host-header loomex.localhost \
  --api-key wfpk_... \
  --api-secret wfsk_...

loomex-runner run \
  --workflow WORKFLOW_UUID \
  --workspace /srv/my-app \
  --input '{"task":"Create a file named greeting.txt with exactly: hello"}'
```

`--input` is workflow input JSON. Objects are passed as-is; arrays, booleans, numbers, and strings
are wrapped by the server under `value`. Use `@file.json` to read JSON from a file.

Human-input nodes are resolved interactively by default. For non-interactive runs, pass:

```bash
loomex-runner run \
  --workflow WORKFLOW_UUID \
  --workspace /srv/my-app \
  --input '{"task":"ship"}' \
  --human-input '{"prompt":"implement the requested change"}'
```

Running `loomex-runner` with no command starts the same interactive flow as `loomex-runner run`.
It prompts for login when no runner token is configured, then asks for workflow, workspace, and
missing JSON inputs.

## Persistent runner usage

```bash
loomex-runner login \
  --server http://127.0.0.1:28000/api/v1/runner-control \
  --host-header loomex.localhost \
  --token lmxrt_...

loomex-runner workspace add demo /Users/oveysrostami/Codes/Loomex/loomex-runner-demo
loomex-runner doctor
loomex-runner start --workspace demo
```

For one-shot smoke tests:

```bash
loomex-runner start --workspace demo --once
```

## mac app packaging smoke

The Tauri mac app shares the Rust core and CLI config/auth paths. Build a repeatable internal
dogfood artifact with:

```bash
scripts/mac_packaging_smoke.sh
```

The script assembles `Loomex.app`, signs it for internal smoke, creates a DMG when macOS tooling is
available, and writes SHA-256 checksums under `target/loomex-tauri-package/`. Use
`LOOMEX_TAURI_SMOKE_LAUNCH=1` on a desktop mac to copy, launch, quit, and relaunch the app. See
[docs/mac-packaging-signing-smoke.md](docs/mac-packaging-signing-smoke.md) for first-run smoke,
Gatekeeper, notarization, and auto-update notes.

## Release security

Install the latest stable Loomex Codex plugin on macOS or Linux with:

```bash
curl -fsSL https://github.com/loomex-app/loomex-plugin/releases/latest/download/install-codex.sh | sh
```

The bootstrap verifies all versioned installation material with Sigstore before
changing Codex. See [plugin/loomex/README.md](plugin/loomex/README.md) for the
two-step verification path and the inherent GitHub TLS trust boundary of
`curl | sh`.

Release artifacts are verified through signed manifests, SHA-256 checksums,
artifact signatures, SBOM entries, and build provenance. `loomex runner release`
provides the local signing and verification helpers used by release CI:

The Codex plugin release is a distinct path: its native macOS/Linux binaries
are unsigned at the platform level and macOS binaries are not notarized. The
published archives, marketplace provenance, and installer are protected with
SHA-256 and keyless Sigstore bundles. macOS Gatekeeper may therefore require a
manual first-run authorization after the user verifies those release records.

```bash
loomex runner release sign-artifact --name loomex-cli-macos-aarch64 --os macos --arch aarch64 --path target/release/loomex --signing-key-env LOOMEX_RELEASE_SIGNING_KEY
loomex runner release sign-manifest --manifest release-manifest.json --signing-key-env LOOMEX_RELEASE_SIGNING_KEY
loomex runner release verify-manifest --manifest release-manifest.signed.json --public-key "$LOOMEX_RELEASE_PUBLIC_KEY"
loomex runner release verify-artifact --manifest release-manifest.signed.json --name loomex-cli-macos-aarch64 --path target/release/loomex --public-key "$LOOMEX_RELEASE_PUBLIC_KEY"
```

See [docs/release-security.md](docs/release-security.md) for release channels,
version pinning, staged rollout, rollback, key rotation, offline updates, partial
downloads, notarization failure handling, and Windows SmartScreen notes.
See [docs/release-channels-installers.md](docs/release-channels-installers.md)
for official Homebrew, direct binary, macOS, Linux, Windows, uninstall, rollback,
and compatibility matrix guidance. The machine-readable release plan is:

```bash
loomex runner release installer-plan --json
loomex runner release validate-compatibility --matrix compatibility.json
```

## Operational readiness

Enterprise runner promotion uses a machine-readable SLO, alert, dashboard,
runbook, and release-gate contract:

```bash
loomex runner ops readiness-plan --json
loomex runner ops release-gate --report operational-readiness-report.json --json
```

See [docs/operational-readiness-slo-runbooks.md](docs/operational-readiness-slo-runbooks.md)
for the official SLO targets, required metrics, runbook drills, capacity plan,
and error-budget gate behavior.

Enterprise release candidates also require acceptance, security, and compliance
sign-off:

```bash
loomex runner ops enterprise-plan --json
loomex runner ops enterprise-signoff --report enterprise-acceptance-report.json --json
```

See [docs/enterprise-acceptance-security-review.md](docs/enterprise-acceptance-security-review.md)
for the checklist, security scope, compliance package, and release-blocking
thresholds.

## Support, debug, and migration

Support can collect redacted diagnostics, export traces, explain local policy,
and migrate safe fields from the legacy `loomex-runner` config without importing
secrets:

```bash
loomex support bundle --json
loomex support diagnostic-request --remote-diagnostic-consent --json
loomex support migrate-legacy --legacy-config ~/.loomex-runner/config.toml --target-config ~/.loomex/config.toml --apply --json
loomex runner doctor --deep --json
loomex trace export RUN_ID --output trace.json --json
loomex policy explain --capability shell.exec --workspace /path/to/workspace --json
```

See [docs/support-debug-migration-tooling.md](docs/support-debug-migration-tooling.md)
for bundle contents, consent requirements, deep diagnostics, trace export,
policy explain, and legacy migration behavior.

## Managed policy

Enterprise admins can enforce organization and project runner policy with
versioned allow/ask/deny layers, rollout, dry-run, rollback, and audit history.
See [docs/managed-policy-admin-controls.md](docs/managed-policy-admin-controls.md)
for API shape and enforcement semantics.

## Runner transport hardening

The production runner data plane remains gRPC-first. WebSocket fallback uses the same generated
protobuf message contract as gRPC with binary frames; REST is management/control only. See
[docs/protocol-hardening-websocket-fallback.md](docs/protocol-hardening-websocket-fallback.md) for
transport negotiation, flow control, metrics, and remaining server-side endpoint work.

## Config

The production `loomex` CLI stores non-secret profile settings at `~/.loomex/config.toml`.
The file is versioned and can be managed with `loomex config get|set|list`.

```toml
configVersion = 2
selectedProfile = "default"

[profiles."default"]
serverUrl = "https://loomex.app"

[profiles."stage"]
serverUrl = "https://stage.loomex.app"

[profiles."local"]
serverUrl = "http://127.0.0.1:28000"
hostHeader = "loomex.localhost"
```

`hostHeader` is accepted only for local/dev profiles. Tokens and stream credentials are stored
outside this config file and are never printed by `loomex config list`.

The Phase 60 command tree is:

```text
loomex login
loomex logout
loomex config get KEY
loomex config set KEY VALUE
loomex config list
loomex org list
loomex org select ORG_ID
loomex project list
loomex project select PROJECT_ID
loomex workflow list
loomex workflow show WORKFLOW_ID
loomex workflow run WORKFLOW_ID --input @input.json
loomex bind .
loomex bind --project PROJECT_ID --workspace PATH
loomex bind list
loomex bind revoke BINDING_ID
loomex runner start
loomex runner stop
loomex runner status
loomex runner logs
loomex runner doctor
loomex runner service unit --platform linux-user|linux-system|windows
loomex runner service install --platform linux-user|linux-system|windows
loomex runner service run --config ~/.loomex/config.toml
loomex approval list
loomex approval approve APPROVAL_ID
loomex approval deny APPROVAL_ID
loomex policy view
loomex policy test --capability NAME --workspace PATH
loomex trace export RUN_ID
```

## Auth, Organization, And Project Selection

Human login uses the browser/device flow:

```bash
loomex login
loomex org list
loomex org select ORG_ID
loomex project list
loomex project select PROJECT_ID
```

Automation can exchange an organization API key and secret without opening a browser:

```bash
loomex login \
  --api-key "$LOOMEX_API_KEY" \
  --api-secret "$LOOMEX_API_SECRET" \
  --organization "$LOOMEX_ORGANIZATION_ID"
```

The selected organization/project are stored per profile in `~/.loomex/config.toml`.
Management tokens are stored outside config under `~/.loomex/credentials/` when OS secure
storage is unavailable; the fallback files are permission-restricted and token material is not
printed by normal commands. Set `LOOMEX_CONFIG_PATH` and `LOOMEX_CREDENTIAL_DIR` to isolate
test or development runs.

The current management contract has token issuance endpoints but no refresh exchange endpoint.
Until that endpoint is added, the CLI fails management calls deterministically with
`AUTH_TOKEN_EXPIRED` when a stored management token is expired or within the clock-skew refresh
window, then asks the user or automation to run `loomex login` again.

Legacy `connect --token ...` and `loomex-runner login --api-key ... --api-secret ...` still work for
compatibility smoke until the compatibility window closes. Production use should run `loomex login`
and create a project/workspace binding with `loomex bind`.

## Windows And Linux Service Mode

Server and workstation installs use the same Rust core as the interactive CLI.
Linux service mode renders systemd units with journal logging:

```bash
loomex runner service unit --platform linux-user
loomex runner service install --platform linux-user
loomex runner service install --platform linux-system --output /tmp/loomex-runner.service
```

Windows service mode renders PowerShell install/uninstall scripts so the final
binary, config path, and log path are explicit and reviewable:

```bash
loomex runner service unit \
  --platform windows \
  --binary 'C:\Program Files\Loomex\loomex.exe' \
  --config 'C:\Users\dev\.loomex\config.toml'
```

Use `--dry-run` to print install artifacts without writing system files. The
service runtime entrypoint is `loomex runner service run --config PATH`; it
uses the shared runtime guard so CLI, service, and desktop app processes cannot
own the same project binding concurrently. The cross-platform build matrix lives
at `.github/workflows/runner-cross-platform.yml` and covers macOS arm64/x64,
Linux x64/arm64, and Windows x64.
