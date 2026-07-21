# Changelog

## 0.1.1 - 2026-07-21

- Use the current Runner Control API for runner identity and health checks.
- Detect legacy runner credentials and guide users through a safe reauthentication flow.
- Keep user and runner credentials separate across the CLI and Tauri clients.
- Allow setup plans to be created locally before authentication is repaired.
- Report durable runner-control health instead of the retired gRPC stream check.
- Use port `28000` for the local backend while preserving customized server URLs.
- Strengthen signed plugin packaging, provenance verification, and rollback-safe installation.

## 0.1.0 - 2026-07-20

- Initial Codex plugin preview with bundled local Runner and human-in-the-loop support.
