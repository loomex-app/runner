# Managed Policy And Admin Controls

Managed runner policy is enforced as a layered allow/ask/deny model:

1. built-in defaults,
2. local config,
3. organization managed policy,
4. project managed policy.

`deny` always wins. Project policy can make organization policy stricter, but it
cannot weaken an organization `ask` or `deny`. It also cannot broaden
organization-managed non-capability restrictions:

- `domains`: project entries must already be allowed by the org allowlist.
- `paths`: project paths must be normalized relative workspace paths that are
  the same path or a child of an org path. Absolute paths, `..`, `.`, drive
  prefixes, and ambiguous path segments are rejected before storage.
- `shellCommands`: project commands must be the same command or a narrower
  command prefix.
- `approvalRequired`: project policy can add required approvals, not remove org
  requirements.
- `artifactRetentionDays`: project retention can be shorter, not longer.

Local config is always lower precedence than managed policy and cannot weaken
managed decisions.

## Policy Shape

The backend admin API stores versioned policy objects:

```json
{
  "capabilities": {
    "shell.exec": "deny",
    "http.request": "ask"
  },
  "domains": ["api.example.com"],
  "paths": ["src", "tests"],
  "shellCommands": ["cargo test", "pnpm test"],
  "approvalRequired": ["fs.write", "shell.exec"],
  "artifactRetentionDays": 30
}
```

The valid decisions are `allow`, `ask`, and `deny`.

## Admin API

Runner control exposes admin endpoints under:

```text
GET  /api/v1/runner-control/runner/v1/admin/policies/
PUT  /api/v1/runner-control/runner/v1/admin/policies/
POST /api/v1/runner-control/runner/v1/admin/policies/dry-run/
POST /api/v1/runner-control/runner/v1/admin/policies/rollback/
```

Requests include `organizationId`; project policies also include `projectId`.
Only the organization owner or a superuser can update managed policy.

Policy updates are versioned. `If-Match` can carry the expected active version
for optimistic concurrency. Every update and rollback emits `policy.changed`.

## Rollout And Stale Runners

Each policy version includes `rolloutPercent`. Runner enforcement calls must
include `runnerRolloutBucket` so the backend can deterministically decide
whether a version applies to that runner. When a managed policy applies, runners
must also report the applied `runnerPolicyVersion` before local actions.

If the version is missing, the backend returns
`MANAGED_POLICY_VERSION_REQUIRED`. If the version is older than the required
active version, it returns `MANAGED_POLICY_STALE` and the runner must refresh
policy before continuing. Out-of-rollout runners do not apply that policy
version and keep the lower/default effective policy until rollout includes their
bucket.

## Rollback

Rollback creates a new active version from a previous version. It does not mutate
the historical version, preserving audit history and deterministic references.
