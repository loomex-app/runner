# Loomex Runner Legacy Compatibility

`loomex-runner` is the Python spike compatibility command. It remains usable
for dev smoke and baseline regression while the Rust/gRPC `loomex` runner is
being built, but it is not the production runner path.

## Compatibility Window

`loomex-runner` stays available until:

1. The Rust/gRPC `loomex` runner passes Phase 50 acceptance.
2. One stable `loomex` CLI release has shipped after that acceptance point.

After this window, `loomex-runner` should be removed from normal developer and
CI flows or retained only as an internal fixture if Technical Lead approval
explicitly keeps it.

## REST Long-Poll Removal

The REST long-poll runner-control path is legacy data plane. It should remain
available only as migration support until Phase 80 migration sign-off.

Production runner execution must use the gRPC bidirectional stream defined by
the runner contracts. REST management APIs remain valid for management-plane
operations.

## Command Mapping

| Legacy command | Production command path |
| --- | --- |
| `loomex-runner login` | `loomex login` |
| `loomex-runner workspace add NAME PATH` | `loomex bind --workspace PATH` |
| `loomex-runner connect --workspace PATH` | `loomex bind --workspace PATH` + `loomex runner start` |
| `loomex-runner run --workflow ID` | `loomex workflow run ID` |
| `loomex-runner start --workspace NAME` | `loomex runner start` |
| `loomex-runner doctor` | `loomex runner doctor` |

The legacy `connect` alias continues to behave like `run` during the
compatibility window because existing smoke scripts may still use it.

## Config Migration

Legacy config:

```text
~/.loomex-runner/config.toml
```

Production config:

```text
~/.loomex/config.toml
```

Migration rules:

- If both files exist, `~/.loomex/config.toml` is the production source of truth.
- If only the legacy file exists, run `loomex login` and recreate bindings with
  `loomex bind`; do not copy runner tokens into the production config.
- If only the production file exists, keep using `loomex`; use `loomex-runner`
  only for compatibility smoke.
- Never print token, API secret, or account secret values during migration.

## User-Facing Warning

Every real `loomex-runner` command prints a deprecation warning to stderr. JSON
stdout remains machine-readable. The warning maps the legacy command to the
production command path and states the compatibility/removal windows without
printing secrets.

## Corner Cases

- User has both old and new config: prefer `~/.loomex/config.toml`.
- Old token is revoked: legacy command fails through normal runner token auth;
  user should run `loomex login`.
- Legacy workspace exists but no production binding exists: create a binding
  with `loomex bind --workspace PATH`.
- CI still invokes `loomex-runner`: keep it only for dev smoke until the
  compatibility window closes, then migrate CI to `loomex workflow run`.

