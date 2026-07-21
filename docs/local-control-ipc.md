# Loomex Local Control IPC v1

The durable Loomex runner exposes a local control socket for Codex, Tauri, and the bundled MCP
adapter. The runner service owns workflow execution; closing an IPC connection does not stop a run
or the service.

## Discovery and security

- Unix/macOS runtime directory: `${LOOMEX_RUNTIME_DIR:-$HOME/.loomex/run}`
- Socket: `control.sock`
- Per-install credential: `control.token`
- Runtime directory mode: `0700`
- Socket and credential modes: `0600`
- The daemon rejects symlinked control paths, insecure ownership/modes, and peers whose effective
  UID differs from the daemon UID.
- Each request carries the credential. Clients must never print or forward it.
- The maximum request line is 1 MiB.

macOS persistence is provided by a per-user LaunchAgent. Linux uses a per-user systemd service.
Windows service packaging remains supported by the existing service manifest; the v1 local control
transport in this document is Unix-domain-socket-only.

## Wire envelope

Messages are UTF-8 newline-delimited JSON. A connection may send one or many requests.

```json
{
  "protocolVersion": "loomex.local-control/v1",
  "id": "request-uuid",
  "authToken": "contents of control.token",
  "method": "workflow.list",
  "params": {}
}
```

Successful response:

```json
{
  "protocolVersion": "loomex.local-control/v1",
  "id": "request-uuid",
  "ok": true,
  "result": {}
}
```

Error response (the connection remains usable):

```json
{
  "protocolVersion": "loomex.local-control/v1",
  "id": "request-uuid",
  "ok": false,
  "error": {
    "code": "LOCAL_CONTROL_PARAMETER_REQUIRED",
    "message": "workflowId is required",
    "retryable": false
  }
}
```

## Methods

The authenticated service implements:

- `ping`, `status`, `setup.status`, `auth.status`
- `workflow.list`, `workflow.show`, `workflow.schema`, `workflow.run`
- `run.list`, `run.get`, `run.wait`, `run.cancel`
- `human.list`, `human.respond`, `approval.list`, `approval.decide`
- `binding.list`, `binding.create`, `binding.revoke`
- `logs.tail`, `doctor`, `runner.status`

Bootstrap methods `setup.plan/apply/rollback`, `auth.start/wait/logout`, `org.list/select`,
`project.list/select`, and `runner.control` return the structured error
`LOCAL_CONTROL_METHOD_REQUIRES_BOOTSTRAP_CLIENT` when sent to an already-authenticated daemon. The
MCP bootstrap client handles those operations using the bundled CLI/runtime installer.

Parameter names are camelCase. Run methods accept `executionId` (`runId` is an alias). Workflow
run uses `workflowId`, optional `inputs`, `sessionId`, and `version`. Human responses use
`requestId` and `payload`; approval decisions use `requestId` and `decision`. Binding create uses
`projectId`, optional `organizationId`/`runnerId`, and `localRootPath`.

The Rust schema is exported from `loomex_core::local_control` as `LocalControlRequest`,
`LocalControlResponse`, `LocalControlError`, `LocalControlPaths`, and
`LOCAL_CONTROL_PROTOCOL_VERSION`.
