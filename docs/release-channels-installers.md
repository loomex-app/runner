# Release Channels And Installers

This document is the Phase 100 installer and channel contract for the
production `loomex` runner surfaces. Every artifact is still governed by the
signed release manifest trust chain in `release-security.md`.

## Channels

| Channel | Purpose | Auto-update |
| --- | --- | --- |
| `stable` | Default production installs. | Allowed after signed manifest, artifact verification, smoke tests, and release approval. |
| `beta` | Opt-in pre-release testing. | Allowed after signed manifest and beta smoke acceptance. |
| `nightly_internal` | Internal dogfood and CI-produced builds. | Allowed for internal profiles only. |
| `enterprise_pinned` | Customer-controlled pinned version. | Not allowed unless enterprise policy changes the pin. |

Use the CLI to print the machine-readable release plan:

```bash
loomex runner release installer-plan --version 1.2.3 --json
```

The JSON schema is `loomex.cli.releaseInstallerPlan/v1`; the embedded plan
schema is `loomex.runner.releaseDistributionPlan/v1`.

## Official Install Paths

### macOS CLI With Homebrew

```bash
brew install loomex/tap/loomex
loomex --version
```

Uninstall removes the binary only:

```bash
brew uninstall loomex
```

It must not delete `~/.loomex`, credentials, support bundles, audit logs, or
local trace data.

### macOS App

Official app distribution is a signed and notarized universal DMG or PKG:

```bash
open Loomex-<version>-universal.dmg
```

or:

```bash
installer -pkg Loomex-<version>-universal.pkg -target CurrentUserHomeDirectory
```

Uninstall removes `Loomex.app` and package receipts only. User config and logs
are preserved unless an explicit purge command is introduced later.

### Linux

Supported Linux artifacts:

- `loomex_<version>_<arch>.deb`
- `loomex-<version>.<arch>.rpm`
- `loomex-<version>-linux-<arch>.tar.gz`

Examples:

```bash
sudo apt install ./loomex_<version>_<arch>.deb
sudo dnf install ./loomex-<version>.<arch>.rpm
tar -xzf loomex-<version>-linux-<arch>.tar.gz
install -m 0755 loomex ~/.local/bin/loomex
```

Uninstall removes package-managed binaries and service files only. It preserves
`~/.loomex`, `/var/lib/loomex`, support bundles, audit logs, and trace data.

### Windows

Supported Windows artifacts:

- `Loomex-<version>-x64.msi`
- `Loomex-<version>-x64.exe`

Examples:

```powershell
msiexec /i Loomex-<version>-x64.msi
Start-Process .\Loomex-<version>-x64.exe
```

Uninstall uses Apps & Features, `msiexec /x`, or the signed uninstaller. It
preserves `%USERPROFILE%\.loomex` unless an explicit purge option is added.

### Direct Binary Download

Direct downloads are supported for CI and locked-down developer machines. The
installer must verify the signed manifest before trusting URLs or checksums,
then verify artifact checksum, size, and signature before placing `loomex` on
`PATH`.

## Compatibility Matrix

The compatibility matrix schema is `loomex.runner.releaseCompatibilityMatrix/v1`.
It records:

- CLI/app version,
- release channel: `stable`, `beta`, `nightly_internal`, or `enterprise_pinned`,
- platform: `macos`, `linux`, `windows`, or `any`,
- architecture: `x86_64`, `aarch64`, `universal`, or `multi`,
- runner protocol version,
- backend minimum version,
- workflow feature compatibility.

Validate a matrix in release automation:

```bash
loomex runner release validate-compatibility --matrix compatibility.json
```

Duplicate CLI/app versions, empty protocol versions, invalid backend versions,
or empty feature names fail validation.

## Upgrade, Rollback, And Downgrade Rules

- Stable-to-stable upgrade requires a signed stable manifest and rollout
  eligibility.
- Rollback requires a signed manifest with `rollback_to_version` present in
  `previous_versions` and an explicit downgrade approval.
- Normal downgrades are rejected to avoid config migration ambiguity.
- Enterprise pinned clients stay on the pinned version until policy changes the
  pin.

## Legacy Binary Deprecation

`loomex-runner` remains compatibility tooling only. The production command is
`loomex`. The compatibility window lasts through enterprise acceptance plus one
stable `loomex` CLI release, then CI smoke paths should migrate to
`loomex workflow run` and `loomex runner ...` commands.
