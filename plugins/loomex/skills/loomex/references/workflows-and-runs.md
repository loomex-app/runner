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

`MANAGEMENT_HTTP_FAILED` and other retryable wait/transport failures mean that
the latest run state is unknown. They do not prove that the durable run was
preserved, cancelled, or failed. Keep the authoritative execution ID and:

1. call `loomex_run_get` for that execution;
2. if the request still has a retryable transport failure, make a small bounded
   number of status attempts with short pauses rather than an unbounded loop;
3. when a non-terminal state is returned, resume bounded `loomex_run_wait`
   calls from the returned sequence and refresh the human and approval inboxes;
4. when a terminal state is returned, report that exact server state and stop
   waiting.

Do not restart the Runner merely because a management request failed three
times. First call `loomex_runner_status`, and use `loomex_runner_doctor` when
status is inconclusive. Recommend a restart only when those authoritative
checks show the local service is unhealthy. A healthy Runner owns reconnect and
replay and should be allowed to recover without a disruptive lifecycle change.
Runner control still requires the impact preview and confirmation described in
[runner-operations.md](runner-operations.md).

Terminal states are `succeeded`, `failed`, and `cancelled`; use the actual
structured state returned by the server. Waiting for human input or approval is
non-terminal. Route those states through the corresponding inbox tools.

A dispatch timeout is a terminal backend result when `loomex_run_get` reports
the run as `failed`: the job was not leased within the dispatch grace period.
Restarting the Runner cannot continue that same terminal execution; a new run
requires a new user request and idempotency key. Do not confuse a retryable
management transport failure with this authoritative terminal result.

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
