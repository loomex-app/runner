# Rust Core Architecture

Status: Phase 20 Task 01 foundation.

The Rust runner workspace is added beside the legacy Python spike in
`loomex-runner/`. The Python package remains the compatibility smoke path; the
Rust workspace is the production foundation for the `loomex` command and future
macOS app.

## Workspace

```text
loomex-runner/
  Cargo.toml
  crates/
    loomex-core/
    loomex-cli/
    loomex-tauri/
```

## Boundary Rules

- `loomex-core` owns auth, config, protocol, binding, policy, approval,
  capability, execution, logs, and redaction modules.
- `loomex-cli` is a thin binary shell. It must not own local execution logic.
- `loomex-tauri` is a UI shell. It must not own local execution logic.
- UI prompts are abstracted by `ApprovalPromptProvider`, with terminal,
  mac-dialog, and non-interactive implementations represented at the boundary.
- Capability execution is abstracted by `CapabilityExecutor` so filesystem,
  shell, git, and HTTP executors can be added without coupling to CLI or mac UI.

## Current Validation

The initial core modules include unit tests for module boundaries, capability
trait mocks, config load/save, cross-platform path normalization, fail-closed
non-interactive approvals, sequence validation, and redaction. Phase 20 Task 02
adds tonic/prost dependencies for the production gRPC data plane.

Cargo-based validation could not be run in this environment because the Rust
toolchain is not installed.

## gRPC Stream Runtime

Phase 20 Task 02 adds the production data-plane foundation:

- `loomex-runner/proto/loomex/runner/v1/runner_stream.proto` mirrors the
  canonical contract proto.
- `loomex-core::grpc` owns tonic codegen, channel construction, TLS endpoint
  config, stream credential validation, required stream metadata, and
  fail-fast proxy detection until tonic proxy transport wiring is added.
- `loomex-core::stream` owns the runner-side stream supervisor: hello/register,
  heartbeat interval tracking, local-provider tool dispatch validation, output
  chunking with per-stream offsets, deadline rejection, protobuf
  started/output/result/error envelope creation, server-close retry
  classification, and network timeout error shaping.
- `loomex-cli` and `loomex-tauri` still contain no execution or gRPC stream
  supervision logic.

## Runner Lifecycle

Phase 20 Task 03 adds deterministic runner lifecycle control in
`loomex-core::lifecycle` and wires it into `loomex-core::stream`:

- displayable runner states for CLI/mac surfaces;
- transition-table validation for auth, binding, connect, run, approval,
  disconnect, pause, update, and error states;
- reconnect backoff with replay resume positions;
- in-flight local tool call tracking;
- cancellation tokens per tool call;
- cancel handling for generated protobuf `ToolCallCancel`;
- server sequence advancement only after accepted messages, with duplicate and
  out-of-order rejection;
- protobuf `ClientAck` construction for accepted or ignored cancel messages;
- graceful shutdown tracking and generated `RunnerDisconnect`;
- emergency stop that blocks new local tool calls.

## Secure Config And Device Identity

Phase 20 Task 04 adds secure identity primitives for the production runner:

- final config path helpers for `~/.loomex/config.toml` plus migration from
  the legacy `~/.loomex-runner/config.toml` location;
- `runner_device_id` stored in config and generated from stable
  org/user/machine/os/arch metadata;
- RunnerDevice upsert logic that reuses the existing device for the same
  org/user/machine tuple and refreshes OS, arch, and runner version metadata;
- token storage abstraction with macOS Keychain as the MVP backend and an
  explicit dev-only secure-file fallback selector;
- separate management and stream token scopes, with stream credentials bound to
  organization, project, runner device, audience, expiry, session, and nonce;
- stream credential refresh window helpers for mid-stream renewal;
- replay nonce tracking and revoke checks for device, binding, and token state;
- no-secret debug/log redaction for configured secrets and token assignments.

Phase 90 Task 03 adds enterprise security controls in `loomex-core::security`:

- configurable egress allowlists, private-network blocking, denied CIDR ranges,
  and redirect target validation for local HTTP execution;
- browser preflight for the initial URL and server-side redirect chain, with
  browser-side JavaScript/meta refresh/client-side navigation documented as a
  residual risk until browser-level interception or OS/network sandboxing;
- workspace sandbox profiles that can deny sensitive prefixes in addition to
  the binding-root and symlink-escape checks;
- child-process environment filtering that removes secret-like variables unless
  explicitly allowed by enterprise policy;
- device posture metadata validation for registration or heartbeat payloads;
- enterprise least-privilege deployment guidance in
  `docs/sandbox-network-device-security.md`.

## Project Runner Binding Model

Phase 30 Task 01 promotes binding from a path string to a core security model:

- `ProjectRunnerBinding` binds organization, project, runner device, and a
  normalized workspace root;
- `WorkspacePath` stores user-visible and normalized roots plus an optional
  runner-computed fingerprint;
- `BindingRegistry` models create/reuse, duplicate same-project path reuse,
  same-path different-project conflict, and revoke behavior;
- workflow-run validation checks binding scope, status, and active project
  permission before local execution starts;
- local tool-call validation requires an active binding and checks requested
  and runner-resolved paths stay inside the binding root, including symlink
  escape detection;
- `ExecutionRegistry::execute_with_binding` provides the binding-aware local
  execution path used by future CLI/mac/provider integrations.

## One Active Runner And Reuse Login

Phase 30 Task 02 adds the MVP active-session rule:

- `RunnerSessionRegistry` enforces one active runner session per
  `ProjectRunnerBinding`;
- a second session for the same binding replaces the previous session, marks it
  `Replaced`, removes it from active dispatch, and records an audit event;
- session heartbeat extends a lease, while missed leases become `Stale` and can
  no longer receive local tool calls;
- server force-disconnect marks the session `Disconnected`, clears active
  ownership, and records the supplied reason;
- reconnecting sessions are represented as `Connecting` and block workflow run
  start until the stream is connected again;
- reusable management token validation keeps bind/run on the same login
  credential generation, while token rotation remains explicit.

## Policy Engine

Phase 30 Task 03 adds policy evaluation before local execution:

- MVP capability taxonomy covers filesystem read/write, shell execution,
  read-only git, and HTTP request capabilities;
- reserved future capabilities such as `git.push`, browser automation, and DB
  queries are policy-compatible but denied as unsupported in the MVP;
- policy layers model built-in defaults, project policy, organization policy,
  enterprise managed policy, and local config, with deny always stronger than a
  weaker local allow;
- evaluator dry-run returns `allow`, `ask`, or `deny` plus source, capability
  support, and reason;
- path decisions are bound to the project runner binding root and deny outside
  workspace or symlink escape attempts;
- network allowlists, sensitive file patterns, and shell command risk
  classification are covered in the shared core evaluator;
- `ExecutionRegistry::execute_with_policy` requires an allow decision before
  invoking a local capability executor.

## Approval Request Flow

Phase 30 Task 04 adds an approval lifecycle that is separate from human input
workflow nodes:

- `ApprovalRequest` captures run id, node id, capability, summary, full request
  details, risk indicators, timeout, policy snapshot, authorized users, and
  requested channel;
- statuses are `Pending`, `Approved`, `Denied`, `Expired`, and `Cancelled`;
- decisions are limited to MVP `AllowOnce` and `Deny`;
- decision handling is idempotent by idempotency key and rejects unauthorized
  users;
- timeout and run-cancel paths produce terminal approval states and audit
  events;
- CLI and mac app approval remain UI-boundary providers, while the shared core
  owns request state, audit events, and prompt payloads.

## File System And Shell Capabilities

Phase 40 Task 02 adds first-class local executors for workspace-bound file and
shell actions:

- `LocalCapabilityExecutor` implements `fs.list`, `fs.read`, `fs.write`,
  `fs.apply_patch`, and `shell.exec`;
- all filesystem paths are relative to the runner workspace root and are
  normalized before access;
- symlink targets are resolved for read/write paths and escape attempts are
  rejected before content is read or changed;
- file reads return encoding, SHA-256, size, binary, and truncation metadata;
- writes support create, overwrite, and append modes with optional expected
  SHA-256 checks;
- patch application returns structured conflicts without partial writes;
- shell execution runs with stdin disabled, filtered environment, bounded cwd,
  timeout, cancellation, output truncation, and redaction metadata;
- on Unix, timeout and cancellation kill the spawned process group so child
  processes are not left running.

## Git Local Provider

Phase 40 Task 03 adds contract-compatible read-only git executors for
workspace-bound repositories:

- `LocalCapabilityExecutor` implements the public MVP capabilities
  `git.status`, `git.diff`, and `git.log`;
- repository discovery starts from the requested workspace-relative path and
  resolves the git top-level before execution;
- discovered repositories must remain inside the binding workspace root, so
  outside-root path attempts and symlink escapes are rejected before git runs;
- status, diff, and log results are returned as structured Rust/JSON payloads
  for workflow routing and AI review;
- branch and detached-HEAD metadata are surfaced through `git.status` instead
  of separate public `git.branch` or `git.remote` capabilities, matching the
  canonical MVP contract;
- diff capture is byte-bounded and reports truncation metadata;
- destructive git actions such as `git.commit` and `git.push` remain reserved
  without an MVP local executor and are denied by default policy.

## HTTP Local Provider

Phase 40 Task 04 adds a contract-compatible local HTTP executor:

- `LocalCapabilityExecutor` implements the public `http.request` capability;
- public JSON input/output follows the canonical `HttpRequestInput` and
  `HttpRequestOutput` schema;
- methods are limited to the MVP HTTP method enum and URLs must be absolute
  `http` or `https` URLs;
- metadata endpoints and unsafe local network ranges are denied before a
  request is sent, while localhost/private IP access remains available for the
  policy layer to approve or ask;
- requests use bounded timeout and response capture, with response body bytes
  kept in non-serialized artifacts and public output exposing `body_ref`;
- redirects are blocked by default for the public executor, with an internal
  helper available for policy-controlled follow behavior;
- Authorization, Cookie, Set-Cookie, and configured secret header names are
  redacted in request/response metadata.
