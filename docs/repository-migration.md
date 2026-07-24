# Repository migration

The active release boundaries are now:

- `loomex-app/loomex-desktop`: Tauri application, desktop runner, and desktop
  installers.
- `loomex-app/loomex-plugin`: Codex plugin, MCP runner, marketplace snapshot,
  and plugin installer.
- `loomex-app/loomex-protocol`: versioned transport-neutral runner contracts.

This repository is transitional. It retains the original combined source and
history while the new repositories are validated in production. It must not be
used to publish new plugin releases. Historical `runner` plugin releases remain
available for rollback, but new plugin installation uses:

```bash
curl -fsSL https://github.com/loomex-app/loomex-plugin/releases/latest/download/install-codex.sh | sh
```

Once at least one desktop release and one plugin release have been installed
and verified on the supported macOS flows, this repository can be made
read-only and archived. Do not delete it before the old release assets and
rollback documentation have been retained.
