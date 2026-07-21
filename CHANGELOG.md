# Changelog

## 0.1.4 - 2026-07-21

- Start first-use onboarding from any natural Loomex request without requiring
  a special setup phrase.
- Make Codex inspect setup, prepare the read-only plan automatically, request
  approval only before applying persistent Runner setup, and then resume the
  original request after authentication, scope selection, and binding.
- Distinguish the bundled verified runtime from durable per-user service state
  in the additive setup-status contract.

## 0.1.3 - 2026-07-21

- Install Cosign 3.1.2 in the release workflow from its official binary with a
  pinned SHA-256 checksum, avoiding the unavailable legacy detached-signature
  asset while preserving keyless Sigstore signing and verification.

## 0.1.2 - 2026-07-21

- Add a one-command GitHub-hosted Codex installer for macOS and Linux.
- Bootstrap Cosign with pinned checksums and an official pinned Sigstore trust root.
- Preserve transactional upgrades and rollback from legacy local marketplaces.

## 0.1.1 - 2026-07-21

- Use the current Runner Control API for runner identity and health checks.
- Detect legacy runner credentials and guide users through a safe reauthentication flow.
- Keep user and runner credentials separate across the CLI and Tauri clients.
- Allow setup plans to be created locally before authentication is repaired.
- Report durable runner-control health instead of the retired gRPC stream check.
- Use port `28000` for the local backend while preserving customized server URLs.
- Publish the plugin with SHA-256 checksums, source-bound provenance, and keyless Sigstore bundles.
- Record macOS/Linux plugin binaries honestly as unsigned and macOS artifacts as unnotarized.

## 0.1.0 - 2026-07-20

- Initial Codex plugin preview with bundled local Runner and human-in-the-loop support.
