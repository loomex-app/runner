# Loomex mac Packaging, Signing, And Smoke

This document is the Phase 70 Task 04 handoff for repeatable internal macOS
dogfooding builds. It intentionally separates the MVP internal smoke path from
the later public distribution path.

## Internal Dogfood Artifact

Run from the runner workspace:

```bash
scripts/mac_packaging_smoke.sh
```

The script builds `loomex-tauri` and the `loomex` runner CLI, places both
binaries in `Loomex.app/Contents/MacOS`, signs the app, creates a DMG when
`hdiutil` is available, and writes SHA-256 checksums plus a structured manifest:

```text
target/loomex-tauri-package/release/Loomex.app
target/loomex-tauri-package/release/Loomex.dmg
target/loomex-tauri-package/release/SHA256SUMS
target/loomex-tauri-package/release/packaging-smoke.json
```

By default the script uses ad-hoc signing with `codesign --sign -`. That is
enough to verify bundle integrity for internal smoke, but it is not a public
Gatekeeper distribution story.

Use a real internal certificate when available:

```bash
LOOMEX_TAURI_SIGN_IDENTITY="Developer ID Application: Loomex Inc. (TEAMID)" \
  scripts/mac_packaging_smoke.sh
```

Run the install/launch/restart smoke explicitly on a desktop mac:

```bash
LOOMEX_TAURI_SMOKE_LAUNCH=1 scripts/mac_packaging_smoke.sh
```

The launch smoke copies the app to a temporary `Applications` directory, opens
it, terminates it, opens it again, and records the result in
`packaging-smoke.json`.

## First-Run Smoke Checklist

Use a clean macOS user when possible.

1. Build the package with `scripts/mac_packaging_smoke.sh`.
2. Verify `shasum -a 256 -c target/loomex-tauri-package/release/SHA256SUMS`.
3. Open the generated DMG or copy `Loomex.app` into `/Applications`.
4. Launch the app.
5. Start browser login and confirm the system browser opens the verification URL.
6. Complete login and confirm credentials are stored in the system keychain when
   available, or local fallback is reported with a warning.
7. Select a workspace using the native Tauri directory picker.
8. Bind the workspace to a project and confirm the bundled runner starts.
9. Run a workflow and confirm no separate CLI install is required.
10. Quit and relaunch the app; config, binding, and non-secret state must
    survive restart.

## Gatekeeper And Signing Risks

Ad-hoc signed builds can still be blocked by Gatekeeper when distributed outside
the local machine, especially when launched from Downloads or after AirDrop/web
download quarantine. The smoke script records `spctl` output as a warning-level
field because public acceptance requires Developer ID signing and notarization.

For public distribution:

1. Build the release artifact on a controlled macOS signer.
2. Sign with a Developer ID Application certificate and hardened runtime.
3. Submit the DMG with `xcrun notarytool submit --wait`.
4. Staple the ticket with `xcrun stapler staple`.
5. Re-run `codesign --verify --deep --strict`, `spctl --assess --type execute`,
   and checksum validation.
6. Publish the checksum next to the signed DMG.

## Auto-Update Plan

Release signing and manifest verification are implemented in the shared Rust
core and exposed through `loomex runner release`. The installer/update loop is
still owned by release automation. A production update channel should use:

- a signed update manifest,
- signed installer artifacts,
- version monotonicity checks,
- signature verification before install,
- no deletion of `~/.loomex/config.toml`, credentials, runner logs, or binding
  state during update,
- rollback guidance when notarization or signature verification fails.

See [release-security.md](release-security.md) for the signed manifest schema,
release channels, version pinning, staged rollout, rollback, SBOM, provenance,
and key-rotation rules.

## Architecture Notes

- The Tauri bundle identifier is `app.loomex.runner`.
- The bundle targets are `app` and `dmg`.
- The app bundle includes `Contents/MacOS/loomex`; Tauri starts it as
  `loomex runner service run` after bind and before workflow execution.
- The app uses the shared Rust core and the same CLI config/auth paths.
- Secure storage is provided by `SystemCredentialStore`: macOS keychain when
  available, with explicit local-file fallback reporting.
- Browser login uses the existing `login_device_start` command output.
- Workspace selection uses the native Tauri dialog command
  `workspace_pick_directory`.
