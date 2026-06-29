# Support, Debug, And Migration Tooling

Loomex Runner support tooling is designed to diagnose customer issues without
direct shell access to the customer's machine.

## Support Bundle

Create a local support bundle:

```bash
loomex support bundle --output loomex-support-bundle.json --json
```

The bundle contains:

- config summary with sensitive keys redacted
- runner version and protocol version
- OS family, OS, and architecture
- runner status and binding summary
- connectivity checks from doctor
- policy snapshot summary
- recent errors
- redacted logs

The bundle does not include credential files, arbitrary file contents, or
sensitive local files. Free-text logs and JSON metadata are recursively redacted
for token, authorization, password, secret, API key, and cookie patterns.

## Remote Diagnostics

Remote diagnostic preparation requires explicit user consent:

```bash
loomex support diagnostic-request --remote-diagnostic-consent --output loomex-support-bundle.json --json
```

Without `--remote-diagnostic-consent`, the command fails with
`REMOTE_DIAGNOSTIC_CONSENT_REQUIRED`.

## Debug Commands

Use these commands for support triage:

```bash
loomex runner doctor --deep --json
loomex trace export RUN_ID --path ~/.loomex/runner.log.jsonl --output trace.json --json
loomex policy explain --capability shell.exec --workspace /path/to/workspace --json
```

`runner doctor --deep` adds proxy and transport-fallback diagnostics. Proxy
configuration is surfaced explicitly because gRPC proxy support fails fast until
transport proxy support is wired.

`trace export` filters local logs by run ID and redacts secrets before writing
or printing the export.

`policy explain` runs the same shared Rust policy evaluator used by runner
execution and returns decision, source, support classification, and reason.

## Legacy Migration

Plan or apply migration from the legacy Python spike config:

```bash
loomex support migrate-legacy \
  --legacy-config ~/.loomex-runner/config.toml \
  --target-config ~/.loomex/config.toml \
  --apply \
  --deactivate-old-daemon \
  --json
```

Migration imports only safe non-secret fields:

- organization ID
- project ID
- runner ID
- binding ID
- workspace path

If the target config already has a non-empty different value for any imported
field, migration fails with `LEGACY_MIGRATION_TARGET_CONFLICT` and leaves the
target file unchanged. This prevents accidentally replacing an existing project
binding or workspace selection.

Legacy tokens are not imported. Users should run `loomex login` if credentials
are missing or expired. Migration output includes warnings about changed runner
behavior and the old daemon deactivation action. Corrupt legacy config fails
with the original structured config parse error.
