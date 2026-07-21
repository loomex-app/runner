# Workflows and durable runs

## Discover

Use `loomex_workflow_list` with the selected organization/project filters. Use
`loomex_workflow_show` before a run when names collide, inputs are missing, or
local capabilities and approval points need explanation. Pass `workflowId`;
pass optional `version` when the user selected a particular immutable version.
If `version` is omitted, report the version returned by Loomex rather than
claiming one was selected. Do not guess workflow IDs, versions, or schema fields.

## Start

Before `loomex_workflow_run`, confirm:

- workflow ID/version;
- selected project and exact binding;
- supplied inputs, especially secrets or environment names;
- declared local capabilities and known approval points.

Call it with required `workflowId`, `bindingId`, and `idempotencyKey`. Its
optional public fields are `inputs`, `version`, and `sessionId`. Reuse the same
idempotency key only when safely retrying the same request; use a new key for an
intentional new run. Include `version`
only for a deliberately selected workflow version. Include `sessionId` only
when a real Loomex/Codex session ID is already available; never fabricate one
from a task title, run ID, or local process. Use the returned execution ID for
all later calls. A submitted or queued response is not completion.

## Follow and reconnect

Use `loomex_run_wait` for bounded server-side waiting. Preserve the cursor or
sequence it returns and send it back as `afterSequence` so repeated waits do not
replay old events. `timeoutSeconds` is optional and is capped by the tool schema.
Provide short progress updates for long runs. If the connection or Codex
restarts, call `loomex_run_get` with the run ID, then resume waiting from the
returned state.

Terminal states are `succeeded`, `failed`, and `cancelled`; use the actual
structured state returned by the server. Waiting for human input or approval is
non-terminal. Route those states through the corresponding inbox tools.

`loomex_run_list` currently requires `workflowId`; it cannot enumerate every run
in a project. When the user lacks both execution ID and workflow ID, resolve the
workflow first with `loomex_workflow_list`. Then call `loomex_run_list` with the
required `workflowId` and optional `status`, `cursor`, and `limit`, and let the
user choose when multiple runs still match. Do not send `projectId` or an empty
workflow ID to this tool.

## Cancel

Before `loomex_run_cancel`, explain which run will be cancelled and whether a
local action is currently executing. Cancellation may be cooperative. Report
`cancellation_requested` separately from terminal `cancelled` and continue
waiting when the user needs confirmation. Call it with required `executionId`,
a non-empty audit `reason`, and `idempotencyKey`. Reuse the key only to retry
that same cancellation request with the same reason.
