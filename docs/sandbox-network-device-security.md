# Sandbox, Network, And Device Security

This document is the Phase 90 Task 03 hardening baseline for enterprise runner
deployments.

## Least Privilege Runtime

Run Loomex Runner with a dedicated, non-admin OS identity wherever possible:

- macOS: launch the Tauri app or service under the interactive user, use
  hardened runtime for packaged builds, and do not grant Full Disk Access unless
  a customer policy explicitly requires it.
- Linux: run the service as a dedicated user, set `NoNewPrivileges=true`, keep
  the workspace and config directories owned by that user, and apply an
  AppArmor/seccomp profile when the customer environment supports it.
- Windows: install the service under a restricted service account, do not use
  LocalSystem for normal deployments, and grant read/write access only to the
  configured workspace roots and Loomex config/log directories.

Runner child processes receive an empty environment plus explicit variables
from the tool request. Secret-like variables such as `API_KEY`, `TOKEN`,
`SECRET`, `PASSWORD`, and `AUTHORIZATION` are removed unless the enterprise
policy explicitly permits that variable name for child execution.

## Workspace Sandbox

The shared core enforces workspace-relative paths before filesystem, shell, git,
test, browser artifact, and database file operations. Enterprise sandbox
profiles can add denied workspace prefixes such as `secrets`, `.ssh`, or
`vendor/private`. A denied prefix blocks the prefix itself and every child path.

The sandbox rejects:

- absolute paths;
- drive-letter or UNC-style paths;
- `..` traversal;
- symlink escapes outside the bound workspace;
- ambiguous path segments with whitespace.

## Network Controls

HTTP execution always blocks local metadata/link-local targets before any
request is sent. Enterprise deployments can additionally configure:

- egress domain allowlists;
- explicit denied CIDR ranges;
- localhost enable/disable;
- private network enable/disable.

Redirect targets are validated before following redirects, so a public endpoint
cannot redirect the runner into a metadata or denied private address.
`browser.playwright` applies the same network policy to the requested URL. When
an enterprise network policy is configured, the runner performs a bounded
preflight request with redirect validation before launching Playwright; if the
redirect chain cannot be validated, navigation fails closed.
This browser preflight covers only the initial URL and server-side HTTP redirect
chain. Browser-side JavaScript redirects, meta refresh/meta redirect behavior,
client-side navigation, and subresource requests after Playwright launches are a
residual risk until browser-level request interception or OS/network sandbox
egress controls enforce the same policy inside the browser process.

Git operations are sandbox-aware. When a sandbox profile denies workspace
prefixes, repo-wide `git.status`, `git.diff`, and `git.log` are not allowed
because they can expose denied paths in output. `git.diff` and `git.log` must be
scoped to pathspecs that pass the same sandbox validation as filesystem access.

Corporate proxy settings remain transport-specific. A proxy that is unsupported
by a transport must fail fast with a structured error instead of silently
ignoring the proxy configuration.

## Device Security

Runner devices have stable device ids scoped by organization/user/machine/OS/arch
metadata. Normal login/upsert cannot resurrect a revoked device. Stream
credentials are short-lived and bound to organization, project, runner device,
runner session, audience, expiry, and nonce.

Device posture metadata should be sent with registration or heartbeat payloads:

- runner device id;
- OS and architecture;
- runner version;
- secure storage status;
- active sandbox profile name;
- whether network policy was enforced;
- collection timestamp.

If a device, binding, or token is revoked, the stream credential path rejects new
connections and the active stream must disconnect or fail within the configured
refresh/reconnect window.

## Validation Checklist

- denied CIDR range is blocked;
- allowlisted domain works;
- metadata endpoint and redirect-to-metadata are blocked;
- revoked device cannot reconnect;
- stream token rotation preserves short-lived expiry;
- workspace and sandbox path escapes are blocked;
- secret environment variables are absent from child processes unless explicitly
  allowed.
