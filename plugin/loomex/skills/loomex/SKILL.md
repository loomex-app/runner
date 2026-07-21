---
name: loomex
description: Use Loomex from Codex to set up its durable local Runner, authenticate, select organizations and projects, bind local workspaces, browse and run workflows, follow long-running runs, respond to human-in-the-loop requests, decide approvals, inspect status and logs, or repair and roll back Runner setup.
---

# Loomex

Use the Loomex MCP tools as the control surface. Keep the backend and durable
Runner as the source of truth; do not attempt to reproduce workflow execution
inside the Codex task.

## Route the request

- Setup, upgrade, repair, or uninstall/rollback: read
  [setup-and-auth.md](references/setup-and-auth.md).
- Organization, project, or local workspace binding: read
  [workspace-binding.md](references/workspace-binding.md).
- Browse workflows, start a run, wait, cancel, or resume after reconnect: read
  [workflows-and-runs.md](references/workflows-and-runs.md).
- Human input or approval: read
  [human-and-approvals.md](references/human-and-approvals.md).
- Health, control, diagnostics, or logs: read
  [runner-operations.md](references/runner-operations.md).
- Before any write or sensitive output, follow [safety.md](references/safety.md).
- For component ownership and lifetime guarantees, read
  [architecture.md](references/architecture.md).

Read every reference needed for the user's request before calling its tools.

## Baseline behavior

1. For a normal first interaction, call `loomex_setup_status`, then
   `loomex_auth_status`. Do not rerun setup when the installation is healthy.
2. If setup is required, use plan then apply. Show the plan and obtain the
   user's approval before persistent service or filesystem changes.
3. Reuse the selected organization, project, and existing binding when they
   unambiguously match the current workspace. Never silently widen a binding.
4. Before running, use `loomex_workflow_show` to confirm inputs and local
   capabilities when the workflow or parameters are ambiguous.
5. Treat the ID returned by `loomex_workflow_run` as authoritative. Follow it
   with `loomex_run_wait`; do not run shell commands to imitate its nodes.
6. When a wait returns a human request or approval, present the exact prompt,
   choices, consequences, and run context. Submit only the user's decision.
7. A closed Codex app cannot surface new prompts. The durable Runner keeps the
   run alive and the backend retains pending work. On reconnect, query the run
   and pending inboxes, and explain this boundary honestly.

## Tool inventory

- Setup: `loomex_setup_status`, `loomex_setup_plan`, `loomex_setup_apply`,
  `loomex_setup_rollback`
- Authentication: `loomex_auth_status`, `loomex_auth_start`,
  `loomex_auth_wait`, `loomex_auth_logout`
- Scope: `loomex_org_list`, `loomex_org_select`, `loomex_project_list`,
  `loomex_project_select`
- Bindings: `loomex_binding_list`, `loomex_binding_create`,
  `loomex_binding_revoke`
- Workflows: `loomex_workflow_list`, `loomex_workflow_show`,
  `loomex_workflow_run`
- Runs: `loomex_run_list`, `loomex_run_get`, `loomex_run_wait`,
  `loomex_run_cancel`
- Human requests: `loomex_human_list`, `loomex_human_respond`
- Approvals: `loomex_approval_list`, `loomex_approval_decide`
- Runner: `loomex_runner_status`, `loomex_runner_control`,
  `loomex_runner_doctor`, `loomex_runner_logs`

Never invent a tool name or infer success from transport success alone. Read the
structured result and report any partial, waiting, rejected, or rollback state.
