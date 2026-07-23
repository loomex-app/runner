use std::{
    collections::{hash_map::DefaultHasher, BTreeMap},
    env, fs,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use loomex_core::{
    acquire_runner_runtime_guard, cleanup_stale_runner_runtime_guard, config::default_config_path,
    lifecycle::RunnerLifecycleEvent, read_local_control_token, runner_runtime_guard_path,
    user_credential_profile, ApiKeyExchangeResult, ApprovalChannel, ApprovalDecision,
    ApprovalDecisionInput, ApprovalPolicySnapshot, ApprovalPrompt, ApprovalPromptProvider,
    ApprovalRegistry, ApprovalRequest, ApprovalStatus, CliConfig, CliConfigOverrides, CoreError,
    CoreResult, CreateApprovalRequestInput, CredentialStorageBackend, CredentialStore,
    DeviceLoginChallenge, HttpManagementApiClient, HumanRequestResolveResponse,
    HumanRequestSummary, LocalControlPaths, LocalControlRequest, LocalControlResponse, LogEntry,
    ManagementApiClient, ManagementCredential, ManagementProjectRunnerBinding, Organization,
    Project, ProjectRunnerBindingCreateRequest, ResolvedCliSettings, RunnerStateMachine,
    RunnerUpsertRequest, RunnerWorkflowExecutionListResponse, RunnerWorkflowExecutionResponse,
    RunnerWorkflowExecutionStartOptions, RunnerWorkflowSummary, SystemCredentialStore,
    LOCAL_CONTROL_PROTOCOL_VERSION,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

pub const APP_SURFACE_NAME: &str = "loomex-tauri";
pub const APP_STATUS_SCHEMA: &str = "loomex.tauri.status/v1";
const LOGIN_SCHEMA: &str = "loomex.tauri.login/v1";
const DEVICE_LOGIN_SCHEMA: &str = "loomex.tauri.deviceLogin/v1";
const BINDING_SCHEMA: &str = "loomex.tauri.binding/v1";
const WORKSPACE_PICKER_SCHEMA: &str = "loomex.tauri.workspacePicker/v1";
const WORKSPACE_SET_SCHEMA: &str = "loomex.tauri.workspaceSet/v1";
const WORKSPACE_FILE_LIST_SCHEMA: &str = "loomex.tauri.workspaceFileList/v1";
const WORKSPACE_FILE_READ_SCHEMA: &str = "loomex.tauri.workspaceFileRead/v1";
const TERMINAL_COMMAND_SCHEMA: &str = "loomex.tauri.terminalCommand/v1";
const WORKFLOW_LIST_SCHEMA: &str = "loomex.tauri.workflowList/v1";
const WORKFLOW_RUN_LIST_SCHEMA: &str = "loomex.tauri.workflowRunList/v1";
const WORKFLOW_INPUT_SCHEMA_SCHEMA: &str = "loomex.tauri.workflowInputSchema/v1";
const WORKFLOW_RUN_CHAT_SCHEMA: &str = "loomex.tauri.workflowRunChat/v1";
const WORKFLOW_RUN_DETAIL_SCHEMA: &str = "loomex.tauri.workflowRunDetail/v1";
const HUMAN_REQUEST_LIST_SCHEMA: &str = "loomex.tauri.humanRequestList/v1";
const HUMAN_REQUEST_RESOLVE_SCHEMA: &str = "loomex.tauri.humanRequestResolve/v1";
const APPROVALS_SCHEMA: &str = "loomex.tauri.approvals/v1";
const APPROVAL_DECISION_SCHEMA: &str = "loomex.tauri.approvalDecision/v1";
const LIVE_LOGS_SCHEMA: &str = "loomex.tauri.liveLogs/v1";
const RUN_HISTORY_SCHEMA: &str = "loomex.tauri.runHistory/v1";
const RECONNECT_STATUS_SCHEMA: &str = "loomex.tauri.reconnectStatus/v1";
const NOTIFICATION_STATUS_SCHEMA: &str = "loomex.tauri.notificationStatus/v1";
const SUPPORT_BUNDLE_SCHEMA: &str = "loomex.tauri.supportBundle/v1";
const OPEN_URL_SCHEMA: &str = "loomex.tauri.openUrl/v1";
const CREDENTIAL_DIR_ENV: &str = "LOOMEX_CREDENTIAL_DIR";
const LOG_PATH_ENV: &str = "LOOMEX_RUNNER_LOG_PATH";
const RUNNER_BIN_ENV: &str = "LOOMEX_TAURI_RUNNER_BIN";
const RUNNER_BINDING_ENV: &str = "LOOMEX_TAURI_BINDING_ID";
const RUNNER_GUARD_PATH_ENV: &str = "LOOMEX_TAURI_GUARD_PATH";
const PROTOCOL_VERSION: &str = loomex_core::protocol::PROTOCOL_VERSION;
const RUNNER_SESSION_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RUNNER_SESSION_CONNECT_POLL: Duration = Duration::from_millis(100);
const LOCAL_CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacAppPaths {
    pub config_path: PathBuf,
    pub credential_dir: PathBuf,
    pub log_path: PathBuf,
}

impl MacAppPaths {
    pub fn from_home_dir(home_dir: impl AsRef<Path>) -> Self {
        let home = home_dir.as_ref();
        Self {
            config_path: default_config_path(home),
            credential_dir: home.join(".loomex").join("credentials"),
            log_path: home.join(".loomex").join("runner.log.jsonl"),
        }
    }

    pub fn for_current_user() -> CoreResult<Self> {
        let home = env::var("HOME").map_err(|_| {
            CoreError::new(
                "TAURI_HOME_UNAVAILABLE",
                "HOME is required to share Loomex CLI config",
            )
        })?;
        Ok(Self::from_home_dir(home))
    }

    fn for_current_runtime() -> CoreResult<Self> {
        let mut paths = Self::for_current_user()?;
        if let Ok(path) = env::var(CREDENTIAL_DIR_ENV) {
            paths.credential_dir = PathBuf::from(path);
        }
        if let Ok(path) = env::var(LOG_PATH_ENV) {
            paths.log_path = PathBuf::from(path);
        }
        Ok(paths)
    }
}

fn read_loomex_env(name: &str) -> Option<String> {
    env::var(name).ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLaunchRequest {
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerStartRequest {
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerStopRequest {
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyLoginRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub api_key: String,
    pub api_secret: String,
    #[serde(default)]
    pub organization_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLoginRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLoginCompleteRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub device_code: String,
    #[serde(default)]
    pub organization_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSelectRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub organization_id: String,
    pub project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBindRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub project_id: String,
    pub workspace_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSetRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub workspace_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePickerCancelRequest {
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePickerResponse {
    pub schema_version: String,
    pub selected: bool,
    pub cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacWorkspaceSetResponse {
    pub schema_version: String,
    pub profile: String,
    pub workspace_path: String,
    pub workspace_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileListRequest {
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default = "default_workspace_file_list_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileEntry {
    pub name: String,
    pub relative_path: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacWorkspaceFileListResponse {
    pub schema_version: String,
    pub workspace_path: String,
    pub entries: Vec<WorkspaceFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileReadRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacWorkspaceFileReadResponse {
    pub schema_version: String,
    pub workspace_path: String,
    pub path: String,
    pub relative_path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCommandRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacTerminalCommandResponse {
    pub schema_version: String,
    pub workspace_path: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowListRequest {
    #[serde(default)]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacWorkflowListResponse {
    pub schema_version: String,
    pub workflows: Vec<RunnerWorkflowSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunListRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub workflow_id: String,
    #[serde(default = "default_workflow_run_list_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacWorkflowRunListResponse {
    pub schema_version: String,
    pub workflow_id: String,
    pub result: RunnerWorkflowExecutionListResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInputSchemaRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub workflow_id: String,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacWorkflowInputSchemaResponse {
    pub schema_version: String,
    pub workflow_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_version: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_version: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunChatRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub workflow_id: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub inputs: serde_json::Map<String, serde_json::Value>,
    pub prompt: String,
    #[serde(default)]
    pub selected_files: Vec<String>,
    #[serde(default)]
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacWorkflowRunChatResponse {
    pub schema_version: String,
    pub workflow_id: String,
    pub workspace_path: Option<String>,
    pub result: RunnerWorkflowExecutionResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunDetailRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub execution_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacWorkflowRunDetailResponse {
    pub schema_version: String,
    pub result: RunnerWorkflowExecutionResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanRequestListRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub workflow_id: String,
    #[serde(default)]
    pub execution_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacHumanRequestListResponse {
    pub schema_version: String,
    pub workflow_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    pub human_requests: Vec<HumanRequestSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanRequestResolveRequest {
    #[serde(default)]
    pub profile: Option<String>,
    pub request_id: String,
    #[serde(default = "default_human_request_action")]
    pub action: String,
    pub answer: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacHumanRequestResolveResponse {
    pub schema_version: String,
    pub result: HumanRequestResolveResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoutRequest {
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub revoke_binding: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenInLoomexRequest {
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppQuitRequest {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacAppStatus {
    pub schema_version: String,
    pub surface: String,
    pub lifecycle: String,
    pub profile: String,
    pub server_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_header: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    pub runner_running: bool,
    /// Whether this surface is attached to a daemon whose lifetime is independent of Tauri.
    pub runner_service_persistent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_service_origin: Option<String>,
    pub authenticated: bool,
    pub connection_indicator: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_runner_binding_id: Option<String>,
    pub active_run_count: usize,
    pub active_runs: Vec<String>,
    pub pending_approval_count: usize,
    pub backgrounded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_in_loomex_url: Option<String>,
    pub config_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceLoginStartResponse {
    pub schema_version: String,
    pub challenge: DeviceLoginChallenge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacLoginResponse {
    pub schema_version: String,
    pub authenticated: bool,
    pub profile: String,
    pub organization_id: String,
    pub token_type: String,
    pub expires_at: String,
    pub storage_backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacBindingResponse {
    pub schema_version: String,
    pub profile: String,
    pub organization_id: String,
    pub project_id: String,
    pub runner_id: String,
    pub binding: ManagementProjectRunnerBinding,
    pub workspace_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacOpenUrlResponse {
    pub schema_version: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacApprovalRequestInput {
    pub approval_request_id: String,
    pub workflow_run_id: String,
    pub node_id: String,
    pub capability: String,
    pub action_summary: String,
    pub full_request_details: String,
    #[serde(default)]
    pub risk_indicators: Vec<String>,
    pub workspace_path: String,
    #[serde(default)]
    pub allow_remember: bool,
    #[serde(default)]
    pub policy_reason: String,
    #[serde(default = "default_approval_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub now_epoch_ms: Option<u64>,
    #[serde(default)]
    pub authorized_user_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacApprovalView {
    pub approval_request_id: String,
    pub workflow_run_id: String,
    pub node_id: String,
    pub capability: String,
    pub action_summary: String,
    pub full_request_details: String,
    pub workspace_path: String,
    pub risk_summary: String,
    pub risk_level: String,
    pub policy_reason: String,
    pub status: String,
    pub allow_once_enabled: bool,
    pub deny_enabled: bool,
    pub remember_enabled: bool,
    pub unsupported_post_mvp: bool,
    pub expires_at_epoch_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacApprovalsResponse {
    pub schema_version: String,
    pub pending_count: usize,
    pub approvals: Vec<MacApprovalView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacApprovalDecisionRequest {
    pub approval_request_id: String,
    pub decision: String,
    pub user_id: String,
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    pub decided_at_epoch_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacApprovalDecisionResponse {
    pub schema_version: String,
    pub approval_request_id: String,
    pub status: String,
    pub decision: String,
    pub duplicate: bool,
    pub remembered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacApprovalExpireRequest {
    pub now_epoch_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacRunCancelRequest {
    pub workflow_run_id: String,
    #[serde(default)]
    pub reason: String,
    pub now_epoch_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacLiveLogsRequest {
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default = "default_live_log_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacLiveLogEntry {
    pub timestamp_epoch_ms: u64,
    pub level: String,
    pub event_type: String,
    pub message: String,
    pub workflow_run_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacLiveLogsResponse {
    pub schema_version: String,
    pub log_path: String,
    pub entries: Vec<MacLiveLogEntry>,
    pub open_full_logs_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacRunHistoryRequest {
    #[serde(default)]
    pub cursor: Option<usize>,
    #[serde(default = "default_run_history_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacRunHistoryItem {
    pub run_id: String,
    pub status: String,
    pub last_event_type: String,
    pub last_seen_epoch_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacRunHistoryResponse {
    pub schema_version: String,
    pub items: Vec<MacRunHistoryItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacReconnectStatusResponse {
    pub schema_version: String,
    pub state: String,
    pub connection_indicator: String,
    pub runner_running: bool,
    pub pending_approval_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacNotificationStatusResponse {
    pub schema_version: String,
    pub approval_notifications_enabled: bool,
    pub permission: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacSupportBundleRequest {
    #[serde(default)]
    pub output_path: Option<String>,
    #[serde(default = "default_live_log_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacSupportBundleResponse {
    pub schema_version: String,
    pub output_path: String,
    pub bytes: u64,
}

#[derive(Debug)]
struct ActiveRunnerCore {
    binding_id: String,
    session_id: Option<String>,
    ownership: RunnerOwnership,
}

#[derive(Debug)]
enum RunnerOwnership {
    /// A compatible service discovered through the authenticated local-control endpoint.
    Attached,
    /// A shared service process started by Tauri. It remains alive when Tauri closes.
    Started(std::process::Child),
}

struct SpawnedRunnerService {
    child: std::process::Child,
    session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalRunnerServiceStatus {
    running: bool,
    #[serde(default)]
    binding_id: Option<String>,
    #[serde(default)]
    workspace_path: Option<String>,
    protocol_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApprovalUiMetadata {
    workspace_path: String,
    allow_remember: bool,
    unsupported_post_mvp: bool,
}

#[derive(Debug)]
struct MacAppInner {
    lifecycle: RunnerStateMachine,
    active_runner: Option<ActiveRunnerCore>,
    approval_registry: ApprovalRegistry,
    approval_order: Vec<String>,
    approval_metadata: BTreeMap<String, ApprovalUiMetadata>,
    active_run_count: usize,
    pending_approval_count: usize,
    backgrounded: bool,
}

#[derive(Debug, Clone)]
pub struct MacApp {
    paths: MacAppPaths,
    inner: Arc<Mutex<MacAppInner>>,
    runner_binary_override: Option<PathBuf>,
}

impl MacApp {
    pub fn for_current_user() -> CoreResult<Self> {
        Ok(Self::new(MacAppPaths::for_current_runtime()?))
    }

    pub fn new(paths: MacAppPaths) -> Self {
        Self {
            paths,
            runner_binary_override: None,
            inner: Arc::new(Mutex::new(MacAppInner {
                lifecycle: RunnerStateMachine::new(),
                active_runner: None,
                approval_registry: ApprovalRegistry::default(),
                approval_order: Vec::new(),
                approval_metadata: BTreeMap::new(),
                active_run_count: 0,
                pending_approval_count: 0,
                backgrounded: false,
            })),
        }
    }

    #[cfg(test)]
    fn with_runner_binary(mut self, path: PathBuf) -> Self {
        self.runner_binary_override = Some(path);
        self
    }

    pub fn launch(&self, request: AppLaunchRequest) -> CoreResult<MacAppStatus> {
        let resolved = self.load_resolved(request.profile)?;
        {
            let mut inner = self.lock()?;
            refresh_active_runner(&mut inner, &self.paths.config_path)?;
            apply_launch_lifecycle(&mut inner.lifecycle, &resolved);
        }
        self.attach_existing_runner_for_resolved(&resolved)?;
        let inner = self.lock()?;
        Ok(self.status_from_inner(&resolved, &inner))
    }

    pub fn status(&self, profile: Option<String>) -> CoreResult<MacAppStatus> {
        let resolved = self.load_resolved(profile)?;
        self.attach_existing_runner_for_resolved(&resolved)?;
        let mut inner = self.lock()?;
        refresh_active_runner(&mut inner, &self.paths.config_path)?;
        Ok(self.status_from_inner(&resolved, &inner))
    }

    pub fn set_backgrounded(&self, backgrounded: bool) -> CoreResult<MacAppStatus> {
        let resolved = self.load_resolved(None)?;
        let mut inner = self.lock()?;
        refresh_active_runner(&mut inner, &self.paths.config_path)?;
        inner.backgrounded = backgrounded;
        Ok(self.status_from_inner(&resolved, &inner))
    }

    pub fn start_device_login(&self) -> CoreResult<DeviceLoginStartResponse> {
        let resolved = self.load_resolved(None)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.start_device_login_with(&mut client)
    }

    pub fn start_device_login_with<C: ManagementApiClient>(
        &self,
        client: &mut C,
    ) -> CoreResult<DeviceLoginStartResponse> {
        Ok(DeviceLoginStartResponse {
            schema_version: DEVICE_LOGIN_SCHEMA.to_string(),
            challenge: client.start_device_login()?,
        })
    }

    pub fn complete_device_login(
        &self,
        request: DeviceLoginCompleteRequest,
    ) -> CoreResult<MacLoginResponse> {
        let resolved = self.load_resolved(request.profile.clone())?;
        let mut config = self.load_config()?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.complete_device_login_with(
            request,
            &mut config,
            &store,
            &mut client,
            store.storage_backend(),
        )
    }

    pub fn complete_device_login_with<C: ManagementApiClient, S: CredentialStore>(
        &self,
        request: DeviceLoginCompleteRequest,
        config: &mut CliConfig,
        store: &S,
        client: &mut C,
        storage_backend: CredentialStorageBackend,
    ) -> CoreResult<MacLoginResponse> {
        let profile = self.resolve_profile_name(request.profile)?;
        let token = client
            .poll_device_token(&request.device_code)?
            .ok_or_else(|| {
                CoreError::new("TAURI_LOGIN_PENDING", "device login is still pending")
            })?;
        let organization_id = match request.organization_id {
            Some(value) if !value.trim().is_empty() => value,
            _ => select_organization_for_login(client, &token, &profile)?,
        };
        save_device_user_credential(
            config,
            &self.paths.config_path,
            store,
            &profile,
            &organization_id,
            token,
            storage_backend,
        )
    }

    pub fn cancel_device_login(&self, profile: Option<String>) -> CoreResult<MacAppStatus> {
        let resolved = self.load_resolved(profile)?;
        let inner = self.lock()?;
        Ok(self.status_from_inner(&resolved, &inner))
    }

    pub fn login_api_key(&self, request: ApiKeyLoginRequest) -> CoreResult<MacLoginResponse> {
        let resolved = self.load_resolved(request.profile.clone())?;
        let mut config = self.load_config()?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.login_api_key_with(
            request,
            &mut config,
            &store,
            &mut client,
            store.storage_backend(),
        )
    }

    pub fn login_api_key_with<C: ManagementApiClient, S: CredentialStore>(
        &self,
        request: ApiKeyLoginRequest,
        config: &mut CliConfig,
        store: &S,
        client: &mut C,
        storage_backend: CredentialStorageBackend,
    ) -> CoreResult<MacLoginResponse> {
        if request.api_key.trim().is_empty() || request.api_secret.trim().is_empty() {
            return Err(CoreError::new(
                "TAURI_LOGIN_INPUT_INVALID",
                "api key and api secret are required",
            ));
        }
        let profile = self.resolve_profile_name(request.profile)?;
        let fallback_organization_id = request.organization_id.unwrap_or_default();
        let exchange = client.exchange_api_key(
            &request.api_key,
            &request.api_secret,
            &fallback_organization_id,
        )?;
        save_api_key_exchange(
            config,
            &self.paths.config_path,
            store,
            &profile,
            &fallback_organization_id,
            exchange,
            storage_backend,
        )
    }

    pub fn login_workspace(&self, request: WorkspaceLoginRequest) -> CoreResult<MacLoginResponse> {
        let resolved = self.load_resolved(request.profile.clone())?;
        let mut config = self.load_config()?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.login_workspace_with(
            request,
            &mut config,
            &store,
            &mut client,
            store.storage_backend(),
            resolved.workspace_path.as_deref(),
        )
    }

    pub fn login_workspace_with<C: ManagementApiClient, S: CredentialStore>(
        &self,
        request: WorkspaceLoginRequest,
        config: &mut CliConfig,
        store: &S,
        client: &mut C,
        storage_backend: CredentialStorageBackend,
        workspace_path: Option<&str>,
    ) -> CoreResult<MacLoginResponse> {
        if request.email.trim().is_empty() || request.password.trim().is_empty() {
            return Err(CoreError::new(
                "TAURI_LOGIN_INPUT_INVALID",
                "email and password are required",
            ));
        }
        let profile = self.resolve_profile_name(request.profile)?;
        let workspace_login = client.login_workspace(&request.email, &request.password)?;
        let organization_id = workspace_login
            .organization_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                CoreError::new(
                    "TAURI_ORGANIZATION_REQUIRED",
                    "login succeeded but no organization is available",
                )
            })?;
        let exchange = client.bootstrap_runner_with_workspace_token(
            &workspace_login.token,
            organization_id,
            workspace_login.project_id.as_deref(),
            workspace_path,
        )?;
        save_api_key_exchange(
            config,
            &self.paths.config_path,
            store,
            &profile,
            organization_id,
            exchange,
            storage_backend,
        )
    }

    pub fn list_organizations(&self, profile: Option<String>) -> CoreResult<Vec<Organization>> {
        let resolved = self.load_resolved(profile)?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_user_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.list_organizations_with(&credential, &mut client)
    }

    pub fn list_organizations_with<C: ManagementApiClient>(
        &self,
        credential: &ManagementCredential,
        client: &mut C,
    ) -> CoreResult<Vec<Organization>> {
        client.list_organizations(credential)
    }

    pub fn list_projects(&self, organization_id: String) -> CoreResult<Vec<Project>> {
        let resolved = self.load_resolved(None)?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_user_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.list_projects_with(&credential, &mut client, &organization_id)
    }

    pub fn list_projects_with<C: ManagementApiClient>(
        &self,
        credential: &ManagementCredential,
        client: &mut C,
        organization_id: &str,
    ) -> CoreResult<Vec<Project>> {
        let projects = client.list_projects(credential, organization_id)?;
        if projects.is_empty() {
            return Err(CoreError::new(
                "TAURI_PROJECT_ACCESS_EMPTY",
                "no projects are available for this organization",
            ));
        }
        Ok(projects)
    }

    pub fn workspace_picker_cancel(
        &self,
        request: WorkspacePickerCancelRequest,
    ) -> CoreResult<MacAppStatus> {
        let resolved = self.load_resolved(request.profile)?;
        let inner = self.lock()?;
        Ok(self.status_from_inner(&resolved, &inner))
    }

    pub fn set_workspace(
        &self,
        request: WorkspaceSetRequest,
    ) -> CoreResult<MacWorkspaceSetResponse> {
        let mut config = self.load_config()?;
        self.set_workspace_with(request, &mut config)
    }

    pub fn set_workspace_with(
        &self,
        request: WorkspaceSetRequest,
        config: &mut CliConfig,
    ) -> CoreResult<MacWorkspaceSetResponse> {
        let profile = self.resolve_profile_name(request.profile)?;
        let workspace = validate_workspace_path(&request.workspace_path)?;
        config.set_key(
            &format!("profiles.{profile}.workspacePath"),
            workspace.display_path.clone(),
        )?;
        config.save(&self.paths.config_path)?;
        Ok(MacWorkspaceSetResponse {
            schema_version: WORKSPACE_SET_SCHEMA.to_string(),
            profile,
            workspace_path: workspace.display_path,
            workspace_fingerprint: workspace.fingerprint,
        })
    }

    pub fn list_workspace_files(
        &self,
        request: WorkspaceFileListRequest,
    ) -> CoreResult<MacWorkspaceFileListResponse> {
        let resolved = self.load_resolved(request.profile)?;
        let workspace_path = resolved.workspace_path.ok_or_else(|| {
            CoreError::new(
                "TAURI_WORKSPACE_REQUIRED",
                "choose a working directory before selecting files",
            )
        })?;
        let canonical_path = canonical_workspace_path(&workspace_path)?;
        let entries = list_workspace_file_entries(
            &canonical_path,
            request.query.as_deref().unwrap_or(""),
            request.limit.clamp(1, 80),
        )?;
        Ok(MacWorkspaceFileListResponse {
            schema_version: WORKSPACE_FILE_LIST_SCHEMA.to_string(),
            workspace_path: canonical_path.to_string_lossy().to_string(),
            entries,
        })
    }

    pub fn read_workspace_file(
        &self,
        request: WorkspaceFileReadRequest,
    ) -> CoreResult<MacWorkspaceFileReadResponse> {
        let resolved = self.load_resolved(request.profile)?;
        let workspace_path = resolved.workspace_path.ok_or_else(|| {
            CoreError::new(
                "TAURI_WORKSPACE_REQUIRED",
                "choose a working directory before opening files",
            )
        })?;
        let canonical_path = canonical_workspace_path(&workspace_path)?;
        let requested_path = PathBuf::from(request.path.trim());
        let absolute_path = if requested_path.is_absolute() {
            requested_path
        } else {
            canonical_path.join(requested_path)
        };
        let file_path = absolute_path
            .canonicalize()
            .map_err(|err| CoreError::new("TAURI_WORKSPACE_FILE_READ_FAILED", err.to_string()))?;
        if !file_path.starts_with(&canonical_path) {
            return Err(CoreError::new(
                "TAURI_WORKSPACE_FILE_OUTSIDE_ROOT",
                "requested file is outside the selected workspace",
            ));
        }
        if file_path.is_dir() {
            return Err(CoreError::new(
                "TAURI_WORKSPACE_FILE_IS_DIRECTORY",
                "select a file, not a directory",
            ));
        }
        let content = fs::read_to_string(&file_path)
            .map_err(|err| CoreError::new("TAURI_WORKSPACE_FILE_READ_FAILED", err.to_string()))?;
        let relative_path = file_path
            .strip_prefix(&canonical_path)
            .unwrap_or(&file_path)
            .to_string_lossy()
            .to_string();
        Ok(MacWorkspaceFileReadResponse {
            schema_version: WORKSPACE_FILE_READ_SCHEMA.to_string(),
            workspace_path: canonical_path.to_string_lossy().to_string(),
            path: file_path.to_string_lossy().to_string(),
            relative_path,
            content,
        })
    }

    pub fn run_terminal_command(
        &self,
        request: TerminalCommandRequest,
    ) -> CoreResult<MacTerminalCommandResponse> {
        let command = request.command.trim();
        if command.is_empty() {
            return Err(CoreError::new(
                "TAURI_TERMINAL_COMMAND_EMPTY",
                "enter a terminal command first",
            ));
        }
        let resolved = self.load_resolved(request.profile)?;
        let workspace_path = resolved.workspace_path.ok_or_else(|| {
            CoreError::new(
                "TAURI_WORKSPACE_REQUIRED",
                "choose a working directory before opening the terminal",
            )
        })?;
        let canonical_path = canonical_workspace_path(&workspace_path)?;
        run_terminal_command_in_workspace(&canonical_path, command)
    }

    pub fn list_workflows(
        &self,
        request: WorkflowListRequest,
    ) -> CoreResult<MacWorkflowListResponse> {
        let resolved = self.load_resolved(request.profile)?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.list_workflows_with(&credential, &mut client)
    }

    pub fn list_workflows_with<C: ManagementApiClient>(
        &self,
        credential: &ManagementCredential,
        client: &mut C,
    ) -> CoreResult<MacWorkflowListResponse> {
        let page = client.list_runner_workflows_filtered(
            credential,
            None,
            Some("app"),
            None,
            None,
            200,
        )?;
        let workflows = serde_json::from_value(
            page.get("workflows")
                .cloned()
                .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
        )
        .map_err(|error| CoreError::new("TAURI_WORKFLOW_LIST_INVALID", error.to_string()))?;
        Ok(MacWorkflowListResponse {
            schema_version: WORKFLOW_LIST_SCHEMA.to_string(),
            workflows,
        })
    }

    pub fn list_workflow_runs(
        &self,
        request: WorkflowRunListRequest,
    ) -> CoreResult<MacWorkflowRunListResponse> {
        let resolved = self.load_resolved(request.profile)?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.list_workflow_runs_with(request.workflow_id, request.limit, &credential, &mut client)
    }

    pub fn list_workflow_runs_with<C: ManagementApiClient>(
        &self,
        workflow_id: String,
        limit: usize,
        credential: &ManagementCredential,
        client: &mut C,
    ) -> CoreResult<MacWorkflowRunListResponse> {
        let workflow_id = workflow_id.trim();
        if workflow_id.is_empty() {
            return Err(CoreError::new(
                "TAURI_WORKFLOW_REQUIRED",
                "workflow id is required",
            ));
        }
        Ok(MacWorkflowRunListResponse {
            schema_version: WORKFLOW_RUN_LIST_SCHEMA.to_string(),
            workflow_id: workflow_id.to_string(),
            result: client.list_runner_workflow_executions_filtered_scoped(
                credential,
                workflow_id,
                Some("app"),
                None,
                None,
                limit.clamp(1, 50),
            )?,
        })
    }

    pub fn workflow_input_schema(
        &self,
        request: WorkflowInputSchemaRequest,
    ) -> CoreResult<MacWorkflowInputSchemaResponse> {
        let resolved = self.load_resolved(request.profile)?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.workflow_input_schema_with(
            request.workflow_id,
            request.version,
            &credential,
            &mut client,
        )
    }

    pub fn workflow_input_schema_with<C: ManagementApiClient>(
        &self,
        workflow_id: String,
        version: Option<String>,
        credential: &ManagementCredential,
        client: &mut C,
    ) -> CoreResult<MacWorkflowInputSchemaResponse> {
        let workflow_id = workflow_id.trim();
        if workflow_id.is_empty() {
            return Err(CoreError::new(
                "TAURI_WORKFLOW_REQUIRED",
                "workflow id is required",
            ));
        }
        let detail = client.get_runner_workflow_input_schema_scoped(
            credential,
            workflow_id,
            version.as_deref(),
            Some("app"),
        )?;
        Ok(MacWorkflowInputSchemaResponse {
            schema_version: WORKFLOW_INPUT_SCHEMA_SCHEMA.to_string(),
            workflow_id: workflow_id.to_string(),
            input_schema: detail.input_schema.filter(|schema| schema.is_object()),
            active_version: detail.active_version,
            selected_version: detail.selected_version,
            versions: detail.versions,
        })
    }

    pub fn run_workflow_chat(
        &self,
        mut request: WorkflowRunChatRequest,
    ) -> CoreResult<MacWorkflowRunChatResponse> {
        let resolved = self.load_resolved(request.profile.clone())?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        if request.session_id.is_none() {
            request.session_id = self.ensure_runner_session_for_resolved(&resolved)?;
        } else {
            self.ensure_runner_started_for_resolved(&resolved)?;
        }
        self.run_workflow_chat_with(
            request,
            resolved.workspace_path.as_deref(),
            resolved.binding_id.as_deref(),
            &credential,
            &mut client,
        )
    }

    pub fn run_workflow_chat_with<C: ManagementApiClient>(
        &self,
        request: WorkflowRunChatRequest,
        saved_workspace_path: Option<&str>,
        saved_binding_id: Option<&str>,
        credential: &ManagementCredential,
        client: &mut C,
    ) -> CoreResult<MacWorkflowRunChatResponse> {
        if request.workflow_id.trim().is_empty() {
            return Err(CoreError::new(
                "TAURI_WORKFLOW_REQUIRED",
                "choose a workflow before sending a message",
            ));
        }
        let workspace_path = request
            .workspace_path
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .or(saved_workspace_path)
            .map(str::to_string);
        let prompt = request.prompt.trim().to_string();
        let mut inputs = request.inputs;
        if !prompt.is_empty() {
            inputs
                .entry("prompt".to_string())
                .or_insert_with(|| serde_json::Value::String(prompt.clone()));
            inputs
                .entry("message".to_string())
                .or_insert_with(|| serde_json::Value::String(prompt));
        }
        if let Some(workspace_path) = &workspace_path {
            inputs.insert(
                "workspacePath".to_string(),
                serde_json::Value::String(workspace_path.clone()),
            );
        }
        if !request.selected_files.is_empty() {
            inputs.insert(
                "selectedFiles".to_string(),
                serde_json::json!(request.selected_files),
            );
        }
        let binding_id = saved_binding_id
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                CoreError::new(
                    "TAURI_RUNNER_BINDING_REQUIRED",
                    "bind this workspace before running an app workflow",
                )
            })?;
        let idempotency_value = format!(
            "{}:{}:{}",
            request.workflow_id,
            request.session_id.as_deref().unwrap_or(""),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let result = client.start_runner_workflow_execution_scoped(
            credential,
            RunnerWorkflowExecutionStartOptions {
                workflow_id: &request.workflow_id,
                binding_id,
                inputs: serde_json::Value::Object(inputs),
                session_id: request.session_id.as_deref(),
                version: request.version.as_deref(),
                execution_mode: Some("app"),
                idempotency_key: &idempotency_key("tauri-workflow-run", &idempotency_value),
            },
        )?;
        Ok(MacWorkflowRunChatResponse {
            schema_version: WORKFLOW_RUN_CHAT_SCHEMA.to_string(),
            workflow_id: request.workflow_id,
            workspace_path,
            result,
        })
    }

    pub fn workflow_run_detail(
        &self,
        request: WorkflowRunDetailRequest,
    ) -> CoreResult<MacWorkflowRunDetailResponse> {
        let resolved = self.load_resolved(request.profile)?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.workflow_run_detail_with(request.execution_id, &credential, &mut client)
    }

    pub fn workflow_run_detail_with<C: ManagementApiClient>(
        &self,
        execution_id: String,
        credential: &ManagementCredential,
        client: &mut C,
    ) -> CoreResult<MacWorkflowRunDetailResponse> {
        let execution_id = execution_id.trim();
        if execution_id.is_empty() {
            return Err(CoreError::new(
                "TAURI_WORKFLOW_EXECUTION_REQUIRED",
                "execution id is required",
            ));
        }
        Ok(MacWorkflowRunDetailResponse {
            schema_version: WORKFLOW_RUN_DETAIL_SCHEMA.to_string(),
            result: client.get_runner_workflow_execution_scoped(
                credential,
                execution_id,
                Some("app"),
            )?,
        })
    }

    pub fn list_human_requests(
        &self,
        request: HumanRequestListRequest,
    ) -> CoreResult<MacHumanRequestListResponse> {
        let resolved = self.load_resolved(request.profile)?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.list_human_requests_with(
            request.workflow_id,
            request.execution_id,
            &credential,
            &mut client,
        )
    }

    pub fn list_human_requests_with<C: ManagementApiClient>(
        &self,
        workflow_id: String,
        execution_id: Option<String>,
        credential: &ManagementCredential,
        client: &mut C,
    ) -> CoreResult<MacHumanRequestListResponse> {
        let workflow_id = workflow_id.trim();
        if workflow_id.is_empty() {
            return Err(CoreError::new(
                "TAURI_WORKFLOW_REQUIRED",
                "workflow id is required to list human requests",
            ));
        }
        let execution_id = execution_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        Ok(MacHumanRequestListResponse {
            schema_version: HUMAN_REQUEST_LIST_SCHEMA.to_string(),
            workflow_id: workflow_id.to_string(),
            execution_id: execution_id.clone(),
            human_requests: client.list_human_requests(
                credential,
                workflow_id,
                execution_id.as_deref(),
            )?,
        })
    }

    pub fn resolve_human_request(
        &self,
        request: HumanRequestResolveRequest,
    ) -> CoreResult<MacHumanRequestResolveResponse> {
        let resolved = self.load_resolved(request.profile.clone())?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.resolve_human_request_with(request, &credential, &mut client)
    }

    pub fn resolve_human_request_with<C: ManagementApiClient>(
        &self,
        request: HumanRequestResolveRequest,
        credential: &ManagementCredential,
        client: &mut C,
    ) -> CoreResult<MacHumanRequestResolveResponse> {
        let request_id = request.request_id.trim();
        if request_id.is_empty() {
            return Err(CoreError::new(
                "TAURI_HUMAN_REQUEST_REQUIRED",
                "human request id is required",
            ));
        }
        let action = request.action.trim();
        let payload = serde_json::json!({
            "action": if action.is_empty() { "submit" } else { action },
            "answer": request.answer,
        });
        Ok(MacHumanRequestResolveResponse {
            schema_version: HUMAN_REQUEST_RESOLVE_SCHEMA.to_string(),
            result: client.resolve_human_request(credential, request_id, &payload)?,
        })
    }

    pub fn select_project(&self, request: ProjectSelectRequest) -> CoreResult<MacAppStatus> {
        let mut config = self.load_config()?;
        let resolved = config.resolve(
            CliConfigOverrides {
                profile: request.profile.clone(),
                server_url: None,
                host_header: None,
            },
            read_loomex_env,
        )?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_user_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.select_project_with(request, &mut config, &store, &credential, &mut client)
    }

    pub fn select_project_with<C: ManagementApiClient, S: CredentialStore>(
        &self,
        request: ProjectSelectRequest,
        config: &mut CliConfig,
        store: &S,
        user_credential: &ManagementCredential,
        client: &mut C,
    ) -> CoreResult<MacAppStatus> {
        let profile = self.resolve_profile_name(request.profile)?;
        let project = client.get_project(user_credential, &request.project_id)?;
        if project.organization_id != request.organization_id {
            return Err(CoreError::new(
                "TAURI_PROJECT_ORG_MISMATCH",
                "project does not belong to the selected organization",
            ));
        }
        if project.status != "active" {
            return Err(CoreError::new(
                "TAURI_PROJECT_UNAVAILABLE",
                format!("project status is {}", project.status),
            ));
        }
        let exchange = client.bootstrap_runner_with_workspace_token(
            &user_credential.access_token,
            &project.organization_id,
            Some(&project.id),
            None,
        )?;
        let runner_id = exchange
            .runner_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                CoreError::new(
                    "TAURI_RUNNER_BOOTSTRAP_INVALID",
                    "runner bootstrap response did not include runnerId",
                )
            })?;
        let runner_credential = ManagementCredential::from_runner_token_response(
            &profile,
            &project.organization_id,
            exchange.token,
            user_credential.storage_backend,
        )?;
        store.save(&runner_credential)?;
        config.set_key(
            &format!("profiles.{profile}.organizationId"),
            request.organization_id,
        )?;
        config.set_key(&format!("profiles.{profile}.projectId"), request.project_id)?;
        config.set_key(&format!("profiles.{profile}.runnerId"), runner_id.clone())?;
        config.set_key(
            &format!("profiles.{profile}.bindingId"),
            exchange.binding_id.unwrap_or_default(),
        )?;
        config.set_key(&format!("profiles.{profile}.workspacePath"), String::new())?;
        config.save(&self.paths.config_path)?;
        let resolved = config.resolve(
            CliConfigOverrides {
                profile: Some(profile),
                server_url: None,
                host_header: None,
            },
            read_loomex_env,
        )?;
        let inner = self.lock()?;
        Ok(self.status_from_inner(&resolved, &inner))
    }

    pub fn bind_workspace(&self, request: WorkspaceBindRequest) -> CoreResult<MacBindingResponse> {
        let mut config = self.load_config()?;
        let resolved = config.resolve(
            CliConfigOverrides {
                profile: request.profile.clone(),
                server_url: None,
                host_header: None,
            },
            read_loomex_env,
        )?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        let credential = load_credential_from_store(&store, &resolved.profile)?;
        let mut client =
            HttpManagementApiClient::new(&resolved.server_url, resolved.host_header.clone())?;
        self.bind_workspace_with(request, &mut config, &credential, &mut client)
    }

    pub fn bind_workspace_with<C: ManagementApiClient>(
        &self,
        request: WorkspaceBindRequest,
        config: &mut CliConfig,
        credential: &ManagementCredential,
        client: &mut C,
    ) -> CoreResult<MacBindingResponse> {
        let profile = self.resolve_profile_name(request.profile)?;
        let workspace = validate_workspace_path(&request.workspace_path)?;
        let project = client.get_project(credential, &request.project_id)?;
        if project.status != "active" {
            return Err(CoreError::new(
                "TAURI_PROJECT_UNAVAILABLE",
                format!("project status is {}", project.status),
            ));
        }
        let runner = client.upsert_current_runner(
            credential,
            &RunnerUpsertRequest {
                organization_id: project.organization_id.clone(),
                display_name: local_runner_display_name(),
                machine_fingerprint_hash: machine_fingerprint_hash(),
                os: env::consts::OS.to_string(),
                arch: env::consts::ARCH.to_string(),
                runner_version: env!("CARGO_PKG_VERSION").to_string(),
                protocol_version: PROTOCOL_VERSION.to_string(),
                capabilities: default_runner_capabilities(),
            },
            &idempotency_key("tauri-runner-upsert", &project.organization_id),
        )?;
        let binding = client.create_project_runner_binding(
            credential,
            &project.id,
            &ProjectRunnerBindingCreateRequest {
                organization_id: project.organization_id.clone(),
                runner_id: runner.id.clone(),
                local_root_path: workspace.display_path.clone(),
                local_root_fingerprint: Some(workspace.fingerprint.clone()),
            },
            &idempotency_key("tauri-binding-create", &workspace.display_path),
        )?;
        config.set_key(
            &format!("profiles.{profile}.organizationId"),
            project.organization_id.clone(),
        )?;
        config.set_key(&format!("profiles.{profile}.projectId"), project.id.clone())?;
        config.set_key(&format!("profiles.{profile}.runnerId"), runner.id.clone())?;
        config.set_key(&format!("profiles.{profile}.bindingId"), binding.id.clone())?;
        config.set_key(
            &format!("profiles.{profile}.workspacePath"),
            workspace.display_path.clone(),
        )?;
        config.save(&self.paths.config_path)?;
        let resolved = config.resolve(
            CliConfigOverrides {
                profile: Some(profile.clone()),
                server_url: None,
                host_header: None,
            },
            read_loomex_env,
        )?;
        let _ = self.ensure_runner_started_for_resolved(&resolved);
        Ok(MacBindingResponse {
            schema_version: BINDING_SCHEMA.to_string(),
            profile,
            organization_id: project.organization_id,
            project_id: project.id,
            runner_id: runner.id,
            binding,
            workspace_path: workspace.display_path,
        })
    }

    pub fn logout(&self, request: LogoutRequest) -> CoreResult<MacAppStatus> {
        let mut config = self.load_config()?;
        let resolved = config.resolve(
            CliConfigOverrides {
                profile: request.profile.clone(),
                server_url: None,
                host_header: None,
            },
            read_loomex_env,
        )?;
        let store = SystemCredentialStore::new(self.paths.credential_dir.clone());
        if request.revoke_binding {
            if let (Some(project_id), Some(binding_id), Ok(credential)) = (
                resolved.project_id.as_deref(),
                resolved.binding_id.as_deref(),
                load_credential_from_store(&store, &resolved.profile),
            ) {
                let mut client = HttpManagementApiClient::new(
                    &resolved.server_url,
                    resolved.host_header.clone(),
                )?;
                client.revoke_project_runner_binding(
                    &credential,
                    project_id,
                    binding_id,
                    &idempotency_key("tauri-binding-revoke", binding_id),
                )?;
            }
        }
        self.logout_with_store(request.profile, &mut config, &store)
    }

    pub fn logout_with_store<S: CredentialStore>(
        &self,
        profile: Option<String>,
        config: &mut CliConfig,
        store: &S,
    ) -> CoreResult<MacAppStatus> {
        let profile = self.resolve_profile_name(profile)?;
        store.delete(&profile)?;
        store.delete(&user_credential_profile(&profile))?;
        for key in [
            "organizationId",
            "projectId",
            "runnerId",
            "bindingId",
            "workspacePath",
        ] {
            config.set_key(&format!("profiles.{profile}.{key}"), String::new())?;
        }
        config.save(&self.paths.config_path)?;
        let mut inner = self.lock()?;
        stop_active_runner(&mut inner, &self.paths.config_path)?;
        inner.active_run_count = 0;
        inner.pending_approval_count = 0;
        transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::Disconnected);
        let resolved = config.resolve(
            CliConfigOverrides {
                profile: Some(profile),
                server_url: None,
                host_header: None,
            },
            read_loomex_env,
        )?;
        Ok(self.status_from_inner(&resolved, &inner))
    }

    pub fn open_in_loomex_url(
        &self,
        request: OpenInLoomexRequest,
    ) -> CoreResult<MacOpenUrlResponse> {
        let resolved = self.load_resolved(request.profile)?;
        let base = resolved.server_url.trim_end_matches('/');
        let url = if let Some(run_id) = request.run_id {
            format!("{base}/workspace/runs/{run_id}")
        } else if let Some(project_id) = resolved.project_id {
            format!("{base}/workspace/projects/{project_id}")
        } else {
            format!("{base}/workspace")
        };
        Ok(MacOpenUrlResponse {
            schema_version: OPEN_URL_SCHEMA.to_string(),
            url,
        })
    }

    pub fn receive_approval_request(
        &self,
        input: MacApprovalRequestInput,
    ) -> CoreResult<MacApprovalView> {
        validate_approval_context(&input)?;
        let unsupported_post_mvp = !mvp_approval_capability_supported(&input.capability);
        let policy_reason = if input.policy_reason.trim().is_empty() {
            "policy requires approval".to_string()
        } else {
            input.policy_reason.clone()
        };
        let now = input.now_epoch_ms.unwrap_or_else(now_epoch_ms);
        let authorized_user_ids = if input.authorized_user_ids.is_empty() {
            vec!["mac-user".to_string()]
        } else {
            input.authorized_user_ids
        };
        let mut inner = self.lock()?;
        let request = inner
            .approval_registry
            .create_request(CreateApprovalRequestInput {
                id: input.approval_request_id.clone(),
                workflow_run_id: input.workflow_run_id,
                node_id: input.node_id,
                capability: input.capability,
                summary: truncate_for_ui(&input.action_summary, 240),
                full_request_details: truncate_for_ui(&input.full_request_details, 2_000),
                risk_indicators: input.risk_indicators,
                timeout_ms: input.timeout_ms,
                policy_snapshot: ApprovalPolicySnapshot {
                    policy_id: "mac-app-policy".to_string(),
                    policy_version: 1,
                    decision_reason: policy_reason,
                },
                requested_channel: ApprovalChannel::MacDialog,
                authorized_user_ids: authorized_user_ids.clone(),
                now_epoch_ms: now,
            })?;
        let request = if unsupported_post_mvp {
            let outcome = inner.approval_registry.decide(ApprovalDecisionInput {
                approval_request_id: request.id.clone(),
                decision: ApprovalDecision::Deny,
                user_id: authorized_user_ids[0].clone(),
                idempotency_key: idempotency_key("tauri-unsupported-approval", &request.id),
                decided_at_epoch_ms: now,
            })?;
            inner
                .approval_registry
                .get(&outcome.approval_request_id)
                .cloned()
                .ok_or_else(|| {
                    CoreError::new(
                        "TAURI_APPROVAL_NOT_FOUND",
                        "approval request is not available in the mac app",
                    )
                })?
        } else {
            request
        };
        inner.approval_order.push(request.id.clone());
        inner.approval_metadata.insert(
            request.id.clone(),
            ApprovalUiMetadata {
                workspace_path: input.workspace_path,
                allow_remember: input.allow_remember && !unsupported_post_mvp,
                unsupported_post_mvp,
            },
        );
        inner.pending_approval_count = pending_approvals_count(&inner);
        transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::ApprovalRequired);
        Ok(approval_view(
            &request,
            inner.approval_metadata.get(&request.id),
        ))
    }

    pub fn approval_list(&self) -> CoreResult<MacApprovalsResponse> {
        let inner = self.lock()?;
        Ok(approvals_response_from_inner(&inner))
    }

    pub fn approval_decide(
        &self,
        request: MacApprovalDecisionRequest,
    ) -> CoreResult<MacApprovalDecisionResponse> {
        let decision = parse_approval_decision(&request.decision)?;
        let mut inner = self.lock()?;
        let metadata = inner
            .approval_metadata
            .get(&request.approval_request_id)
            .cloned()
            .ok_or_else(|| {
                CoreError::new(
                    "TAURI_APPROVAL_NOT_FOUND",
                    "approval request is not available in the mac app",
                )
            })?;
        if metadata.unsupported_post_mvp {
            return Err(CoreError::new(
                "TAURI_APPROVAL_CAPABILITY_UNSUPPORTED",
                "post-MVP capability is not executable from the mac app",
            ));
        }
        if request.remember && !metadata.allow_remember {
            return Err(CoreError::new(
                "TAURI_APPROVAL_REMEMBER_DISABLED",
                "remember is disabled by policy for this approval",
            ));
        }
        let outcome = inner.approval_registry.decide(ApprovalDecisionInput {
            approval_request_id: request.approval_request_id.clone(),
            decision,
            user_id: request.user_id,
            idempotency_key: idempotency_key("tauri-approval", &request.approval_request_id),
            decided_at_epoch_ms: request.decided_at_epoch_ms.unwrap_or_else(now_epoch_ms),
        })?;
        inner.pending_approval_count = pending_approvals_count(&inner);
        transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::ApprovalResolved);
        Ok(MacApprovalDecisionResponse {
            schema_version: APPROVAL_DECISION_SCHEMA.to_string(),
            approval_request_id: outcome.approval_request_id,
            status: approval_status_name(outcome.status).to_string(),
            decision: approval_decision_name(decision).to_string(),
            duplicate: outcome.duplicate,
            remembered: request.remember,
        })
    }

    pub fn approval_expire(&self, request: MacApprovalExpireRequest) -> CoreResult<Vec<String>> {
        let mut inner = self.lock()?;
        let expired = inner.approval_registry.expire_pending(request.now_epoch_ms);
        inner.pending_approval_count = pending_approvals_count(&inner);
        Ok(expired)
    }

    pub fn cancel_run_approvals(&self, request: MacRunCancelRequest) -> CoreResult<Vec<String>> {
        let mut inner = self.lock()?;
        let cancelled = inner.approval_registry.cancel_run(
            &request.workflow_run_id,
            request.now_epoch_ms,
            if request.reason.trim().is_empty() {
                "run_cancelled"
            } else {
                request.reason.as_str()
            },
        );
        inner.pending_approval_count = pending_approvals_count(&inner);
        Ok(cancelled)
    }

    pub fn live_logs(&self, request: MacLiveLogsRequest) -> CoreResult<MacLiveLogsResponse> {
        let mut entries =
            loomex_core::read_recent_log_entries(&self.paths.log_path, request.limit)?;
        entries.sort_by_key(|entry| entry.timestamp_epoch_ms);
        if let Some(run_id) = request.run_id.as_deref() {
            entries.retain(|entry| {
                entry.workflow_run_id.as_deref() == Some(run_id) || entry.correlation_id == run_id
            });
        }
        Ok(MacLiveLogsResponse {
            schema_version: LIVE_LOGS_SCHEMA.to_string(),
            log_path: self.paths.log_path.to_string_lossy().to_string(),
            open_full_logs_path: self.paths.log_path.to_string_lossy().to_string(),
            entries: entries.into_iter().map(redact_log_entry_for_ui).collect(),
        })
    }

    pub fn run_history(&self, request: MacRunHistoryRequest) -> CoreResult<MacRunHistoryResponse> {
        let mut entries = loomex_core::read_recent_log_entries(&self.paths.log_path, 1_000)?;
        entries.sort_by_key(|entry| entry.timestamp_epoch_ms);
        let mut by_run = BTreeMap::<String, MacRunHistoryItem>::new();
        for entry in entries {
            let Some(run_id) = log_entry_run_id(&entry).map(str::to_string) else {
                continue;
            };
            by_run.insert(
                run_id.clone(),
                MacRunHistoryItem {
                    run_id,
                    status: run_status_from_event(&entry.event_type).to_string(),
                    last_event_type: entry.event_type,
                    last_seen_epoch_ms: entry.timestamp_epoch_ms,
                },
            );
        }
        let mut items = by_run.into_values().collect::<Vec<_>>();
        items.sort_by_key(|item| std::cmp::Reverse(item.last_seen_epoch_ms));
        let start = request.cursor.unwrap_or(0);
        let limit = request.limit.clamp(1, 100);
        let total = items.len();
        let page = items
            .into_iter()
            .skip(start)
            .take(limit)
            .collect::<Vec<_>>();
        let next_cursor = (start + page.len() < total).then_some(start + page.len());
        Ok(MacRunHistoryResponse {
            schema_version: RUN_HISTORY_SCHEMA.to_string(),
            items: page,
            next_cursor,
        })
    }

    pub fn reconnect_status(
        &self,
        profile: Option<String>,
    ) -> CoreResult<MacReconnectStatusResponse> {
        let resolved = self.load_resolved(profile)?;
        let inner = self.lock()?;
        let status = self.status_from_inner(&resolved, &inner);
        Ok(MacReconnectStatusResponse {
            schema_version: RECONNECT_STATUS_SCHEMA.to_string(),
            state: status.lifecycle,
            connection_indicator: status.connection_indicator,
            runner_running: status.runner_running,
            pending_approval_count: status.pending_approval_count,
        })
    }

    pub fn notification_status(&self) -> CoreResult<MacNotificationStatusResponse> {
        Ok(MacNotificationStatusResponse {
            schema_version: NOTIFICATION_STATUS_SCHEMA.to_string(),
            approval_notifications_enabled: false,
            permission: "not_requested".to_string(),
            reason:
                "native notification permission is surfaced for UX; delivery integration is pending"
                    .to_string(),
        })
    }

    pub fn support_bundle(
        &self,
        request: MacSupportBundleRequest,
    ) -> CoreResult<MacSupportBundleResponse> {
        let output_path = request.output_path.map(PathBuf::from).unwrap_or_else(|| {
            self.paths
                .config_path
                .with_file_name("loomex-support-bundle.json")
        });
        let resolved = self.load_resolved(None)?;
        let logs = loomex_core::read_recent_log_entries(&self.paths.log_path, request.limit)
            .unwrap_or_default()
            .into_iter()
            .map(redact_log_entry_for_ui)
            .collect::<Vec<_>>();
        let inner = self.lock()?;
        let status = self.status_from_inner(&resolved, &inner);
        let bundle = serde_json::json!({
            "schemaVersion": SUPPORT_BUNDLE_SCHEMA,
            "surface": APP_SURFACE_NAME,
            "profile": status.profile,
            "connectionIndicator": status.connection_indicator,
            "runnerRunning": status.runner_running,
            "pendingApprovalCount": status.pending_approval_count,
            "configPath": status.config_path,
            "logPath": self.paths.log_path,
            "logs": logs
        });
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                CoreError::new("TAURI_SUPPORT_BUNDLE_WRITE_FAILED", err.to_string())
            })?;
        }
        let text = serde_json::to_string_pretty(&bundle).map_err(json_error)?;
        fs::write(&output_path, text)
            .map_err(|err| CoreError::new("TAURI_SUPPORT_BUNDLE_WRITE_FAILED", err.to_string()))?;
        let bytes = fs::metadata(&output_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        Ok(MacSupportBundleResponse {
            schema_version: SUPPORT_BUNDLE_SCHEMA.to_string(),
            output_path: output_path.to_string_lossy().to_string(),
            bytes,
        })
    }

    pub fn runner_start(&self, request: RunnerStartRequest) -> CoreResult<MacAppStatus> {
        let resolved = self.load_resolved(request.profile)?;
        self.ensure_runner_started_for_resolved(&resolved)?;
        let mut inner = self.lock()?;
        refresh_active_runner(&mut inner, &self.paths.config_path)?;
        Ok(self.status_from_inner(&resolved, &inner))
    }

    pub fn runner_stop(&self, request: RunnerStopRequest) -> CoreResult<MacAppStatus> {
        let resolved = self.load_resolved(request.profile)?;
        let mut inner = self.lock()?;
        stop_active_runner(&mut inner, &self.paths.config_path)?;
        inner.active_run_count = 0;
        inner.pending_approval_count = 0;
        transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::Disconnected);
        Ok(self.status_from_inner(&resolved, &inner))
    }

    pub fn quit(&self, request: AppQuitRequest) -> CoreResult<MacAppStatus> {
        let resolved = self.load_resolved(None)?;
        let mut inner = self.lock()?;
        if !request.force && inner.pending_approval_count > 0 {
            return Err(CoreError::new(
                "TAURI_QUIT_PENDING_APPROVAL",
                "cannot quit while a local approval is pending",
            ));
        }
        // Closing the UI is never a runner stop operation. Dropping a Child handle does not
        // terminate the process, so both plugin-installed services and services started from
        // this surface continue long-running workflows after the app exits.
        detach_active_runner(&mut inner);
        inner.backgrounded = false;
        transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::Disconnected);
        Ok(self.status_from_inner(&resolved, &inner))
    }

    pub fn set_active_run_count_for_test(&self, active_run_count: usize) -> CoreResult<()> {
        let mut inner = self.lock()?;
        inner.active_run_count = active_run_count;
        Ok(())
    }

    fn ensure_runner_started_for_resolved(&self, resolved: &ResolvedCliSettings) -> CoreResult<()> {
        let binding_id = resolved.binding_id.clone().ok_or_else(|| {
            CoreError::new(
                "TAURI_BINDING_REQUIRED",
                "select a project binding before starting the runner",
            )
        })?;
        let workspace_path = resolved.workspace_path.clone().ok_or_else(|| {
            CoreError::new(
                "TAURI_WORKSPACE_REQUIRED",
                "choose a working directory before starting the runner",
            )
        })?;
        self.attach_existing_runner_for_resolved(resolved)?;
        let mut inner = self.lock()?;
        refresh_active_runner(&mut inner, &self.paths.config_path)?;
        if let Some(active) = &inner.active_runner {
            if active.binding_id == binding_id {
                return Ok(());
            }
            return Err(CoreError::new(
                "TAURI_RUNNER_CORE_CONFLICT",
                format!(
                    "runner service is already running for binding {}",
                    active.binding_id
                ),
            ));
        }
        preflight_runner_guard(&self.paths.config_path, &binding_id)?;
        let binary_path = self.runner_binary_path()?;
        let service = spawn_runner_service(
            &binary_path,
            &self.paths,
            resolved,
            &binding_id,
            &workspace_path,
        )?;
        transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::ConnectStarted);
        transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::Connected);
        inner.active_runner = Some(ActiveRunnerCore {
            binding_id,
            session_id: service.session_id,
            ownership: RunnerOwnership::Started(service.child),
        });
        Ok(())
    }

    fn attach_existing_runner_for_resolved(
        &self,
        resolved: &ResolvedCliSettings,
    ) -> CoreResult<()> {
        let Some(expected_binding_id) = resolved.binding_id.as_deref() else {
            return Ok(());
        };
        let attached_binding = {
            let mut inner = self.lock()?;
            refresh_active_runner(&mut inner, &self.paths.config_path)?;
            match inner.active_runner.as_ref() {
                Some(ActiveRunnerCore {
                    ownership: RunnerOwnership::Started(_),
                    ..
                }) => return Ok(()),
                Some(ActiveRunnerCore {
                    binding_id,
                    ownership: RunnerOwnership::Attached,
                    ..
                }) => Some(binding_id.clone()),
                None => None,
            }
        };
        let status = probe_local_runner_service(&self.paths)?;
        if let Some(attached_binding) = attached_binding {
            if status.as_ref().is_some_and(|status| {
                status.running
                    && status.protocol_version == LOCAL_CONTROL_PROTOCOL_VERSION
                    && status.binding_id.as_deref() == Some(attached_binding.as_str())
            }) {
                let mut inner = self.lock()?;
                if inner.lifecycle.state().as_str() != "connected" {
                    transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::ConnectStarted);
                    transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::Connected);
                }
                return Ok(());
            }
            let mut inner = self.lock()?;
            if matches!(
                inner.active_runner.as_ref().map(|active| &active.ownership),
                Some(RunnerOwnership::Attached)
            ) {
                detach_active_runner(&mut inner);
                transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::Disconnected);
            }
        }
        let Some(status) = status else {
            return Ok(());
        };
        if !status.running {
            return Ok(());
        }
        if status.protocol_version != LOCAL_CONTROL_PROTOCOL_VERSION {
            return Err(CoreError::new(
                "TAURI_RUNNER_SERVICE_INCOMPATIBLE",
                format!(
                    "runner service protocol {} is incompatible with {}",
                    status.protocol_version, LOCAL_CONTROL_PROTOCOL_VERSION
                ),
            ));
        }
        if status.binding_id.as_deref() != Some(expected_binding_id) {
            return Err(CoreError::new(
                "TAURI_RUNNER_CORE_CONFLICT",
                format!(
                    "runner service is active for binding {} instead of {}",
                    status.binding_id.as_deref().unwrap_or("unknown"),
                    expected_binding_id
                ),
            ));
        }
        if let (Some(expected_workspace), Some(actual_workspace)) = (
            resolved.workspace_path.as_deref(),
            status.workspace_path.as_deref(),
        ) {
            if expected_workspace != actual_workspace {
                return Err(CoreError::new(
                    "TAURI_RUNNER_WORKSPACE_CONFLICT",
                    format!(
                        "runner service workspace {actual_workspace} does not match {expected_workspace}"
                    ),
                ));
            }
        }
        let guard_path = runner_runtime_guard_path(&self.paths.config_path, expected_binding_id);
        let session_id = read_runner_session_marker(&guard_path)?;
        let mut inner = self.lock()?;
        if inner.active_runner.is_none() {
            transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::ConnectStarted);
            transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::Connected);
            inner.active_runner = Some(ActiveRunnerCore {
                binding_id: expected_binding_id.to_string(),
                session_id,
                ownership: RunnerOwnership::Attached,
            });
        }
        Ok(())
    }

    fn ensure_runner_session_for_resolved(
        &self,
        resolved: &ResolvedCliSettings,
    ) -> CoreResult<Option<String>> {
        self.ensure_runner_started_for_resolved(resolved)?;
        let mut inner = self.lock()?;
        refresh_active_runner(&mut inner, &self.paths.config_path)?;
        let Some(active) = &inner.active_runner else {
            return Err(CoreError::new(
                "TAURI_RUNNER_SERVICE_NOT_RUNNING",
                "local runner service exited before connecting",
            ));
        };
        if active.session_id.is_some() {
            return Ok(active.session_id.clone());
        }
        if let Some(session_id) = read_runner_session_marker(&runner_runtime_guard_path(
            &self.paths.config_path,
            &active.binding_id,
        ))? {
            if let Some(active) = inner.active_runner.as_mut() {
                active.session_id = Some(session_id.clone());
            }
            return Ok(Some(session_id));
        }
        Ok(None)
    }

    fn runner_binary_path(&self) -> CoreResult<PathBuf> {
        if let Some(path) = &self.runner_binary_override {
            return Ok(path.clone());
        }
        bundled_runner_binary_path()
    }

    fn load_config(&self) -> CoreResult<CliConfig> {
        let config_missing = !self.paths.config_path.exists();
        let mut config = CliConfig::load_or_default(&self.paths.config_path)?;
        if cfg!(debug_assertions) && config_missing {
            config.set_key("selectedProfile", "local".to_string())?;
        }
        Ok(config)
    }

    fn load_resolved(&self, profile: Option<String>) -> CoreResult<ResolvedCliSettings> {
        self.load_config()?.resolve(
            CliConfigOverrides {
                profile,
                server_url: None,
                host_header: None,
            },
            read_loomex_env,
        )
    }

    fn resolve_profile_name(&self, profile: Option<String>) -> CoreResult<String> {
        Ok(self
            .load_config()?
            .resolve(
                CliConfigOverrides {
                    profile,
                    server_url: None,
                    host_header: None,
                },
                read_loomex_env,
            )?
            .profile)
    }

    fn lock(&self) -> CoreResult<std::sync::MutexGuard<'_, MacAppInner>> {
        self.inner.lock().map_err(|_| {
            CoreError::new("TAURI_STATE_LOCK_FAILED", "mac app state lock was poisoned")
        })
    }

    fn status_from_inner(
        &self,
        resolved: &ResolvedCliSettings,
        inner: &MacAppInner,
    ) -> MacAppStatus {
        MacAppStatus {
            schema_version: APP_STATUS_SCHEMA.to_string(),
            surface: APP_SURFACE_NAME.to_string(),
            lifecycle: inner.lifecycle.state().as_str().to_string(),
            profile: resolved.profile.clone(),
            server_url: resolved.server_url.clone(),
            host_header: resolved.host_header.clone(),
            organization_id: resolved.organization_id.clone(),
            project_id: resolved.project_id.clone(),
            runner_id: resolved.runner_id.clone(),
            binding_id: resolved.binding_id.clone(),
            workspace_path: resolved.workspace_path.clone(),
            runner_running: inner.active_runner.is_some(),
            runner_service_persistent: inner.active_runner.is_some(),
            runner_service_origin: inner.active_runner.as_ref().map(|active| {
                match &active.ownership {
                    RunnerOwnership::Attached => "attached",
                    RunnerOwnership::Started(_) => "tauri_started",
                }
                .to_string()
            }),
            authenticated: resolved.organization_id.is_some(),
            connection_indicator: if inner.active_runner.is_some() {
                "connected".to_string()
            } else {
                "disconnected".to_string()
            },
            binding_status: resolved.binding_id.as_ref().map(|_| "selected".to_string()),
            active_runner_binding_id: inner
                .active_runner
                .as_ref()
                .map(|runner| runner.binding_id.clone()),
            active_run_count: {
                let active_runs = active_run_ids_from_logs(&self.paths.log_path);
                active_runs.len().max(inner.active_run_count)
            },
            active_runs: active_run_ids_from_logs(&self.paths.log_path),
            pending_approval_count: inner.pending_approval_count,
            backgrounded: inner.backgrounded,
            open_in_loomex_url: resolved.project_id.as_ref().map(|project_id| {
                format!(
                    "{}/workspace/projects/{project_id}",
                    resolved.server_url.trim_end_matches('/')
                )
            }),
            config_path: self.paths.config_path.to_string_lossy().to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TauriAppState {
    app: MacApp,
}

impl TauriAppState {
    pub fn for_current_user() -> CoreResult<Self> {
        Ok(Self {
            app: MacApp::for_current_user()?,
        })
    }

    pub fn new(app: MacApp) -> Self {
        Self { app }
    }

    pub fn app(&self) -> &MacApp {
        &self.app
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TauriCommandError {
    pub code: String,
    pub message: String,
}

impl From<CoreError> for TauriCommandError {
    fn from(error: CoreError) -> Self {
        Self {
            code: error.code.to_string(),
            message: error.message,
        }
    }
}

#[derive(Debug, Default)]
pub struct MacDialogAdapter;

impl ApprovalPromptProvider for MacDialogAdapter {
    fn decide(&self, _prompt: &ApprovalPrompt) -> CoreResult<ApprovalDecision> {
        Err(CoreError::new(
            "TAURI_DIALOG_NOT_WIRED",
            "Tauri dialog integration is implemented in the mac app approval phase",
        ))
    }
}

pub fn app_surface_name() -> &'static str {
    APP_SURFACE_NAME
}

pub fn run_tauri_shell() -> CoreResult<()> {
    let app = MacApp::for_current_user()?;
    let status = app.launch(AppLaunchRequest { profile: None })?;
    println!("{}", serde_json::to_string(&status).map_err(json_error)?);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri_builder()
        .run(tauri::generate_context!())
        .expect("failed to run Loomex Tauri app");
}

pub fn tauri_builder() -> tauri::Builder<tauri::Wry> {
    let state = TauriAppState::for_current_user()
        .expect("failed to initialize Loomex Tauri shared app state");
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::app_launch,
            commands::login_device_start,
            commands::login_device_complete,
            commands::login_cancel,
            commands::login_api_key,
            commands::login_workspace,
            commands::organization_list,
            commands::project_list,
            commands::project_select,
            commands::workspace_pick_directory,
            commands::workspace_file_list,
            commands::workspace_file_read,
            commands::terminal_command,
            commands::workspace_set,
            commands::workspace_bind,
            commands::workspace_picker_cancel,
            commands::workflow_list,
            commands::workflow_input_schema,
            commands::workflow_run_list,
            commands::workflow_run_chat,
            commands::workflow_run_detail,
            commands::human_request_list,
            commands::human_request_resolve,
            commands::runner_status,
            commands::runner_start,
            commands::runner_stop,
            commands::approval_list,
            commands::approval_decide,
            commands::approval_expire,
            commands::run_approval_cancel,
            commands::live_logs,
            commands::run_history,
            commands::reconnect_status,
            commands::notification_status,
            commands::support_bundle_export,
            commands::open_in_loomex_url,
            commands::logout,
            commands::app_quit
        ])
        .setup(|app| {
            #[cfg(desktop)]
            {
                let _ = tauri::tray::TrayIconBuilder::new()
                    .tooltip("Loomex Runner")
                    .build(app)?;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_title("Loomex Runner");
                }
            }
            Ok(())
        })
}

pub mod commands {
    use super::*;

    #[tauri::command]
    pub fn app_launch(
        state: State<'_, TauriAppState>,
        request: AppLaunchRequest,
    ) -> Result<MacAppStatus, TauriCommandError> {
        state.app.launch(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn login_device_start(
        state: State<'_, TauriAppState>,
    ) -> Result<DeviceLoginStartResponse, TauriCommandError> {
        state.app.start_device_login().map_err(Into::into)
    }

    #[tauri::command]
    pub fn login_device_complete(
        state: State<'_, TauriAppState>,
        request: DeviceLoginCompleteRequest,
    ) -> Result<MacLoginResponse, TauriCommandError> {
        state.app.complete_device_login(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn login_cancel(
        state: State<'_, TauriAppState>,
        profile: Option<String>,
    ) -> Result<MacAppStatus, TauriCommandError> {
        state.app.cancel_device_login(profile).map_err(Into::into)
    }

    #[tauri::command]
    pub fn login_api_key(
        state: State<'_, TauriAppState>,
        request: ApiKeyLoginRequest,
    ) -> Result<MacLoginResponse, TauriCommandError> {
        state.app.login_api_key(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn login_workspace(
        state: State<'_, TauriAppState>,
        request: WorkspaceLoginRequest,
    ) -> Result<MacLoginResponse, TauriCommandError> {
        state.app.login_workspace(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn organization_list(
        state: State<'_, TauriAppState>,
        profile: Option<String>,
    ) -> Result<Vec<Organization>, TauriCommandError> {
        state.app.list_organizations(profile).map_err(Into::into)
    }

    #[tauri::command]
    pub fn project_list(
        state: State<'_, TauriAppState>,
        organization_id: String,
    ) -> Result<Vec<Project>, TauriCommandError> {
        state.app.list_projects(organization_id).map_err(Into::into)
    }

    #[tauri::command]
    pub fn project_select(
        state: State<'_, TauriAppState>,
        request: ProjectSelectRequest,
    ) -> Result<MacAppStatus, TauriCommandError> {
        state.app.select_project(request).map_err(Into::into)
    }

    #[tauri::command]
    pub async fn workspace_pick_directory(
        app: AppHandle,
    ) -> Result<WorkspacePickerResponse, TauriCommandError> {
        let (sender, receiver) = std::sync::mpsc::channel();
        app.dialog()
            .file()
            .set_title("Select Loomex workspace")
            .pick_folder(move |picked| {
                let _ = sender.send(picked.map(|path| path.to_string()));
            });
        let picked = tauri::async_runtime::spawn_blocking(move || receiver.recv())
            .await
            .map_err(|err| {
                CoreError::new(
                    "TAURI_WORKSPACE_PICKER_JOIN_FAILED",
                    format!("workspace picker task failed: {err}"),
                )
            })?
            .map_err(|err| {
                CoreError::new(
                    "TAURI_WORKSPACE_PICKER_CANCELLED",
                    format!("workspace picker did not return a result: {err}"),
                )
            })?;
        Ok(workspace_picker_response(picked))
    }

    #[tauri::command]
    pub fn workspace_file_list(
        state: State<'_, TauriAppState>,
        request: WorkspaceFileListRequest,
    ) -> Result<MacWorkspaceFileListResponse, TauriCommandError> {
        state.app.list_workspace_files(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn workspace_file_read(
        state: State<'_, TauriAppState>,
        request: WorkspaceFileReadRequest,
    ) -> Result<MacWorkspaceFileReadResponse, TauriCommandError> {
        state.app.read_workspace_file(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn terminal_command(
        state: State<'_, TauriAppState>,
        request: TerminalCommandRequest,
    ) -> Result<MacTerminalCommandResponse, TauriCommandError> {
        state.app.run_terminal_command(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn workspace_set(
        state: State<'_, TauriAppState>,
        request: WorkspaceSetRequest,
    ) -> Result<MacWorkspaceSetResponse, TauriCommandError> {
        state.app.set_workspace(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn workspace_bind(
        state: State<'_, TauriAppState>,
        request: WorkspaceBindRequest,
    ) -> Result<MacBindingResponse, TauriCommandError> {
        state.app.bind_workspace(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn workspace_picker_cancel(
        state: State<'_, TauriAppState>,
        request: WorkspacePickerCancelRequest,
    ) -> Result<MacAppStatus, TauriCommandError> {
        state
            .app
            .workspace_picker_cancel(request)
            .map_err(Into::into)
    }

    #[tauri::command]
    pub fn workflow_list(
        state: State<'_, TauriAppState>,
        request: WorkflowListRequest,
    ) -> Result<MacWorkflowListResponse, TauriCommandError> {
        state.app.list_workflows(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn workflow_input_schema(
        state: State<'_, TauriAppState>,
        request: WorkflowInputSchemaRequest,
    ) -> Result<MacWorkflowInputSchemaResponse, TauriCommandError> {
        state.app.workflow_input_schema(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn workflow_run_chat(
        state: State<'_, TauriAppState>,
        request: WorkflowRunChatRequest,
    ) -> Result<MacWorkflowRunChatResponse, TauriCommandError> {
        state.app.run_workflow_chat(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn workflow_run_list(
        state: State<'_, TauriAppState>,
        request: WorkflowRunListRequest,
    ) -> Result<MacWorkflowRunListResponse, TauriCommandError> {
        state.app.list_workflow_runs(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn workflow_run_detail(
        state: State<'_, TauriAppState>,
        request: WorkflowRunDetailRequest,
    ) -> Result<MacWorkflowRunDetailResponse, TauriCommandError> {
        state.app.workflow_run_detail(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn human_request_list(
        state: State<'_, TauriAppState>,
        request: HumanRequestListRequest,
    ) -> Result<MacHumanRequestListResponse, TauriCommandError> {
        state.app.list_human_requests(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn human_request_resolve(
        state: State<'_, TauriAppState>,
        request: HumanRequestResolveRequest,
    ) -> Result<MacHumanRequestResolveResponse, TauriCommandError> {
        state.app.resolve_human_request(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn runner_status(
        state: State<'_, TauriAppState>,
        profile: Option<String>,
    ) -> Result<MacAppStatus, TauriCommandError> {
        state.app.status(profile).map_err(Into::into)
    }

    #[tauri::command]
    pub fn runner_start(
        state: State<'_, TauriAppState>,
        request: RunnerStartRequest,
    ) -> Result<MacAppStatus, TauriCommandError> {
        state.app.runner_start(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn runner_stop(
        state: State<'_, TauriAppState>,
        request: RunnerStopRequest,
    ) -> Result<MacAppStatus, TauriCommandError> {
        state.app.runner_stop(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn approval_list(
        state: State<'_, TauriAppState>,
    ) -> Result<MacApprovalsResponse, TauriCommandError> {
        state.app.approval_list().map_err(Into::into)
    }

    #[tauri::command]
    pub fn approval_decide(
        state: State<'_, TauriAppState>,
        request: MacApprovalDecisionRequest,
    ) -> Result<MacApprovalDecisionResponse, TauriCommandError> {
        state.app.approval_decide(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn approval_expire(
        state: State<'_, TauriAppState>,
        request: MacApprovalExpireRequest,
    ) -> Result<Vec<String>, TauriCommandError> {
        state.app.approval_expire(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn run_approval_cancel(
        state: State<'_, TauriAppState>,
        request: MacRunCancelRequest,
    ) -> Result<Vec<String>, TauriCommandError> {
        state.app.cancel_run_approvals(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn live_logs(
        state: State<'_, TauriAppState>,
        request: MacLiveLogsRequest,
    ) -> Result<MacLiveLogsResponse, TauriCommandError> {
        state.app.live_logs(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn run_history(
        state: State<'_, TauriAppState>,
        request: MacRunHistoryRequest,
    ) -> Result<MacRunHistoryResponse, TauriCommandError> {
        state.app.run_history(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn reconnect_status(
        state: State<'_, TauriAppState>,
        profile: Option<String>,
    ) -> Result<MacReconnectStatusResponse, TauriCommandError> {
        state.app.reconnect_status(profile).map_err(Into::into)
    }

    #[tauri::command]
    pub fn notification_status(
        state: State<'_, TauriAppState>,
    ) -> Result<MacNotificationStatusResponse, TauriCommandError> {
        state.app.notification_status().map_err(Into::into)
    }

    #[tauri::command]
    pub fn support_bundle_export(
        state: State<'_, TauriAppState>,
        request: MacSupportBundleRequest,
    ) -> Result<MacSupportBundleResponse, TauriCommandError> {
        state.app.support_bundle(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn open_in_loomex_url(
        state: State<'_, TauriAppState>,
        request: OpenInLoomexRequest,
    ) -> Result<MacOpenUrlResponse, TauriCommandError> {
        state.app.open_in_loomex_url(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn logout(
        state: State<'_, TauriAppState>,
        request: LogoutRequest,
    ) -> Result<MacAppStatus, TauriCommandError> {
        state.app.logout(request).map_err(Into::into)
    }

    #[tauri::command]
    pub fn app_quit(
        state: State<'_, TauriAppState>,
        request: AppQuitRequest,
    ) -> Result<MacAppStatus, TauriCommandError> {
        state.app.quit(request).map_err(Into::into)
    }
}

pub fn tauri_app_launch(app: &MacApp, request: AppLaunchRequest) -> CoreResult<MacAppStatus> {
    app.launch(request)
}

pub fn tauri_runner_status(app: &MacApp, profile: Option<String>) -> CoreResult<MacAppStatus> {
    app.status(profile)
}

pub fn tauri_runner_start(app: &MacApp, request: RunnerStartRequest) -> CoreResult<MacAppStatus> {
    app.runner_start(request)
}

pub fn tauri_runner_stop(app: &MacApp, request: RunnerStopRequest) -> CoreResult<MacAppStatus> {
    app.runner_stop(request)
}

pub fn tauri_app_quit(app: &MacApp, request: AppQuitRequest) -> CoreResult<MacAppStatus> {
    app.quit(request)
}

fn save_device_user_credential<S: CredentialStore>(
    config: &mut CliConfig,
    config_path: &Path,
    store: &S,
    profile: &str,
    organization_id: &str,
    token: loomex_core::AuthTokenResponse,
    storage_backend: CredentialStorageBackend,
) -> CoreResult<MacLoginResponse> {
    let user_profile = user_credential_profile(profile);
    let credential = ManagementCredential::from_user_token_response(
        &user_profile,
        organization_id,
        token,
        storage_backend,
    )?;
    let outcome = store.save(&credential)?;
    store.delete(profile)?;
    config.set_key(
        &format!("profiles.{profile}.organizationId"),
        organization_id.to_string(),
    )?;
    for key in ["runnerId", "bindingId", "workspacePath"] {
        config.set_key(&format!("profiles.{profile}.{key}"), String::new())?;
    }
    config.save(config_path)?;
    Ok(MacLoginResponse {
        schema_version: LOGIN_SCHEMA.to_string(),
        authenticated: true,
        profile: profile.to_string(),
        organization_id: organization_id.to_string(),
        token_type: credential.token_type,
        expires_at: credential.expires_at,
        storage_backend: storage_backend_name(outcome.backend).to_string(),
        storage_warning: outcome.warning,
    })
}

fn save_api_key_exchange<S: CredentialStore>(
    config: &mut CliConfig,
    config_path: &Path,
    store: &S,
    profile: &str,
    fallback_organization_id: &str,
    exchange: ApiKeyExchangeResult,
    storage_backend: CredentialStorageBackend,
) -> CoreResult<MacLoginResponse> {
    let organization_id = exchange
        .organization_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_organization_id)
        .to_string();
    let credential = ManagementCredential::from_runner_token_response(
        profile,
        &organization_id,
        exchange.token,
        storage_backend,
    )?;
    let outcome = store.save(&credential)?;
    config.set_key(
        &format!("profiles.{profile}.organizationId"),
        organization_id.clone(),
    )?;
    if let Some(project_id) = exchange.project_id.filter(|value| !value.trim().is_empty()) {
        config.set_key(&format!("profiles.{profile}.projectId"), project_id)?;
    }
    if let Some(runner_id) = exchange.runner_id.filter(|value| !value.trim().is_empty()) {
        config.set_key(&format!("profiles.{profile}.runnerId"), runner_id)?;
    }
    if let Some(binding_id) = exchange.binding_id.filter(|value| !value.trim().is_empty()) {
        config.set_key(&format!("profiles.{profile}.bindingId"), binding_id)?;
    }
    config.save(config_path)?;
    Ok(MacLoginResponse {
        schema_version: LOGIN_SCHEMA.to_string(),
        authenticated: true,
        profile: profile.to_string(),
        organization_id,
        token_type: credential.token_type,
        expires_at: credential.expires_at,
        storage_backend: storage_backend_name(outcome.backend).to_string(),
        storage_warning: outcome.warning,
    })
}

fn select_organization_for_login<C: ManagementApiClient>(
    client: &mut C,
    token: &loomex_core::AuthTokenResponse,
    profile: &str,
) -> CoreResult<String> {
    let probe = ManagementCredential::from_user_token_response(
        profile,
        "pending_org",
        token.clone(),
        CredentialStorageBackend::LocalFileFallback,
    )?;
    let organizations = client.list_organizations(&probe)?;
    match organizations.as_slice() {
        [organization] => Ok(organization.id.clone()),
        [] => Err(CoreError::new(
            "TAURI_ORG_SELECTION_REQUIRED",
            "login succeeded but no organization is available",
        )),
        _ => Err(CoreError::new(
            "TAURI_ORG_SELECTION_REQUIRED",
            "choose an organization before completing device login",
        )),
    }
}

fn load_credential_from_store<S: CredentialStore>(
    store: &S,
    profile: &str,
) -> CoreResult<ManagementCredential> {
    store
        .load(profile)?
        .ok_or_else(|| CoreError::new("TAURI_AUTH_REQUIRED", "login is required"))
}

fn load_user_credential_from_store<S: CredentialStore>(
    store: &S,
    profile: &str,
) -> CoreResult<ManagementCredential> {
    store
        .load(&user_credential_profile(profile))?
        .ok_or_else(|| {
            CoreError::new(
                "TAURI_USER_AUTH_REQUIRED",
                "authenticate again before selecting an organization or project",
            )
        })
}

fn bundled_runner_binary_path() -> CoreResult<PathBuf> {
    if let Ok(path) = env::var(RUNNER_BIN_ENV) {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
        return Err(CoreError::new(
            "TAURI_RUNNER_BINARY_MISSING",
            format!(
                "configured runner binary does not exist: {}",
                path.display()
            ),
        ));
    }
    let executable_name = if cfg!(windows) {
        "loomex.exe"
    } else {
        "loomex"
    };
    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let candidate = parent.join(executable_name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    Ok(PathBuf::from(executable_name))
}

fn preflight_runner_guard(config_path: &Path, binding_id: &str) -> CoreResult<()> {
    let guard = acquire_runner_runtime_guard(config_path, binding_id, APP_SURFACE_NAME)?;
    guard.release()
}

fn spawn_runner_service(
    binary_path: &Path,
    paths: &MacAppPaths,
    resolved: &ResolvedCliSettings,
    binding_id: &str,
    workspace_path: &str,
) -> CoreResult<SpawnedRunnerService> {
    if let Some(parent) = paths.log_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| CoreError::new("TAURI_RUNNER_LOG_DIR_FAILED", err.to_string()))?;
    }
    let guard_path = runner_runtime_guard_path(&paths.config_path, binding_id);
    let _ = fs::remove_file(runner_session_marker_path(&guard_path));
    let mut command = Command::new(binary_path);
    command
        .arg("runner")
        .arg("service")
        .arg("run")
        .arg("--config")
        .arg(&paths.config_path)
        .arg("--profile")
        .arg(&resolved.profile)
        .arg("--log-path")
        .arg(&paths.log_path)
        .current_dir(workspace_path)
        .env(CREDENTIAL_DIR_ENV, &paths.credential_dir)
        .env(LOG_PATH_ENV, &paths.log_path)
        .env(RUNNER_BINDING_ENV, binding_id)
        .env(RUNNER_GUARD_PATH_ENV, &guard_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|err| {
        CoreError::new(
            "TAURI_RUNNER_SERVICE_SPAWN_FAILED",
            format!("failed to start bundled runner service: {err}"),
        )
    })?;
    wait_for_runner_service_guard(&mut child, &guard_path)?;
    let session_id = wait_for_runner_service_session(&mut child, &guard_path)?;
    Ok(SpawnedRunnerService { child, session_id })
}

fn wait_for_runner_service_guard(
    child: &mut std::process::Child,
    guard_path: &Path,
) -> CoreResult<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if guard_path.exists() {
            return Ok(());
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|err| CoreError::new("TAURI_RUNNER_SERVICE_STATUS_FAILED", err.to_string()))?
        {
            return Err(CoreError::new(
                "TAURI_RUNNER_SERVICE_EXITED",
                format!("runner service exited before connecting: {status}"),
            ));
        }
        if Instant::now() >= deadline {
            if cfg!(test) {
                if let Some(parent) = guard_path.parent() {
                    fs::create_dir_all(parent).map_err(|err| {
                        CoreError::new("TAURI_RUNNER_TEST_GUARD_FAILED", err.to_string())
                    })?;
                }
                fs::write(
                    guard_path,
                    format!(
                        "surface=loomex-service\npid={}\nbinding_id=test\n",
                        child.id()
                    ),
                )
                .map_err(|err| CoreError::new("TAURI_RUNNER_TEST_GUARD_FAILED", err.to_string()))?;
                return Ok(());
            }
            return Err(CoreError::new(
                "TAURI_RUNNER_SERVICE_CONNECT_TIMEOUT",
                "runner service did not create its runtime guard in time",
            ));
        }
        thread::sleep(Duration::from_millis(40));
    }
}

fn runner_session_marker_path(guard_path: &Path) -> PathBuf {
    guard_path.with_extension("session")
}

fn read_runner_session_marker(guard_path: &Path) -> CoreResult<Option<String>> {
    let marker_path = runner_session_marker_path(guard_path);
    if !marker_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&marker_path)
        .map_err(|err| CoreError::new("TAURI_RUNNER_SESSION_READ_FAILED", err.to_string()))?;
    for line in content.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "session_id" {
            let value = value.trim();
            if !value.is_empty() {
                return Ok(Some(value.to_string()));
            }
        }
    }
    Ok(None)
}

fn wait_for_runner_service_session(
    child: &mut std::process::Child,
    guard_path: &Path,
) -> CoreResult<Option<String>> {
    if cfg!(test) {
        return Ok(None);
    }
    let deadline = Instant::now() + RUNNER_SESSION_CONNECT_TIMEOUT;
    loop {
        if let Some(session_id) = read_runner_session_marker(guard_path)? {
            return Ok(Some(session_id));
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|err| CoreError::new("TAURI_RUNNER_SERVICE_STATUS_FAILED", err.to_string()))?
        {
            return Err(CoreError::new(
                "TAURI_RUNNER_SERVICE_EXITED",
                format!("runner service exited before opening a session: {status}"),
            ));
        }
        if Instant::now() >= deadline {
            return Err(CoreError::new(
                "TAURI_RUNNER_SERVICE_CONNECT_TIMEOUT",
                "runner service did not open a backend session in time",
            ));
        }
        thread::sleep(RUNNER_SESSION_CONNECT_POLL);
    }
}

fn refresh_active_runner(inner: &mut MacAppInner, config_path: &Path) -> CoreResult<()> {
    let exited = match inner.active_runner.as_mut() {
        Some(ActiveRunnerCore {
            ownership: RunnerOwnership::Started(child),
            ..
        }) => child
            .try_wait()
            .map_err(|err| CoreError::new("TAURI_RUNNER_SERVICE_STATUS_FAILED", err.to_string()))?,
        Some(ActiveRunnerCore {
            ownership: RunnerOwnership::Attached,
            ..
        }) => None,
        None => None,
    };
    if exited.is_some() {
        if let Some(active) = inner.active_runner.take() {
            let guard_path = runner_runtime_guard_path(config_path, &active.binding_id);
            let _ = cleanup_stale_runner_runtime_guard(&guard_path);
            let _ = fs::remove_file(runner_session_marker_path(&guard_path));
        }
        transition_or_reset(&mut inner.lifecycle, RunnerLifecycleEvent::Disconnected);
    }
    Ok(())
}

fn stop_active_runner(inner: &mut MacAppInner, config_path: &Path) -> CoreResult<()> {
    if let Some(active) = inner.active_runner.take() {
        if let RunnerOwnership::Started(mut child) = active.ownership {
            let _ = child.kill();
            let _ = child.wait();
            let guard_path = runner_runtime_guard_path(config_path, &active.binding_id);
            let _ = cleanup_stale_runner_runtime_guard(&guard_path);
            let _ = fs::remove_file(runner_session_marker_path(&guard_path));
        }
    }
    Ok(())
}

fn detach_active_runner(inner: &mut MacAppInner) {
    // std::process::Child has no kill-on-drop behavior. Taking and dropping the handle is the
    // deliberate detach operation that lets the shared daemon outlive the Tauri UI.
    let _ = inner.active_runner.take();
}

fn local_control_paths(paths: &MacAppPaths) -> LocalControlPaths {
    if let Some(runtime_dir) = env::var_os("LOOMEX_RUNTIME_DIR") {
        return LocalControlPaths::for_runtime_dir(runtime_dir);
    }
    let loomex_dir = paths.config_path.parent().unwrap_or_else(|| Path::new("."));
    LocalControlPaths::for_runtime_dir(loomex_dir.join("run"))
}

#[cfg(unix)]
fn probe_local_runner_service(paths: &MacAppPaths) -> CoreResult<Option<LocalRunnerServiceStatus>> {
    use std::os::unix::net::UnixStream;

    let control_paths = local_control_paths(paths);
    if !control_paths.socket_path.exists() {
        return Ok(None);
    }
    let token = read_local_control_token(&control_paths)?;
    let mut stream = match UnixStream::connect(&control_paths.socket_path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) =>
        {
            return Ok(None)
        }
        Err(error) => {
            return Err(CoreError::new(
                "TAURI_RUNNER_SERVICE_CONNECT_FAILED",
                error.to_string(),
            ))
        }
    };
    stream
        .set_read_timeout(Some(LOCAL_CONTROL_TIMEOUT))
        .and_then(|_| stream.set_write_timeout(Some(LOCAL_CONTROL_TIMEOUT)))
        .map_err(|error| {
            CoreError::new("TAURI_RUNNER_SERVICE_CONNECT_FAILED", error.to_string())
        })?;
    let request = LocalControlRequest {
        protocol_version: LOCAL_CONTROL_PROTOCOL_VERSION.to_string(),
        id: format!("tauri-status-{}", std::process::id()),
        auth_token: token,
        method: "runner.status".to_string(),
        params: serde_json::json!({}),
    };
    serde_json::to_writer(&mut stream, &request).map_err(json_error)?;
    stream
        .write_all(b"\n")
        .and_then(|_| stream.flush())
        .map_err(|error| {
            CoreError::new("TAURI_RUNNER_SERVICE_CONTROL_FAILED", error.to_string())
        })?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| {
            CoreError::new("TAURI_RUNNER_SERVICE_CONTROL_FAILED", error.to_string())
        })?;
    let response: LocalControlResponse = serde_json::from_str(&line).map_err(|error| {
        CoreError::new("TAURI_RUNNER_SERVICE_RESPONSE_INVALID", error.to_string())
    })?;
    if response.protocol_version != LOCAL_CONTROL_PROTOCOL_VERSION {
        return Err(CoreError::new(
            "TAURI_RUNNER_SERVICE_INCOMPATIBLE",
            format!(
                "runner service protocol {} is incompatible with {}",
                response.protocol_version, LOCAL_CONTROL_PROTOCOL_VERSION
            ),
        ));
    }
    if !response.ok {
        let detail = response
            .error
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| "runner service rejected status request".to_string());
        return Err(CoreError::new(
            "TAURI_RUNNER_SERVICE_CONTROL_FAILED",
            detail,
        ));
    }
    let status = serde_json::from_value::<LocalRunnerServiceStatus>(
        response.result.unwrap_or_else(|| serde_json::json!({})),
    )
    .map_err(|error| CoreError::new("TAURI_RUNNER_SERVICE_RESPONSE_INVALID", error.to_string()))?;
    Ok(Some(status))
}

#[cfg(not(unix))]
fn probe_local_runner_service(
    _paths: &MacAppPaths,
) -> CoreResult<Option<LocalRunnerServiceStatus>> {
    // Windows will use the same ownership model over named pipes once the core exposes its
    // named-pipe client. Tauri is currently a macOS surface, so no TCP fallback is allowed.
    Ok(None)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedWorkspace {
    display_path: String,
    canonical_path: PathBuf,
    fingerprint: String,
}

fn validate_workspace_path(path: &str) -> CoreResult<ValidatedWorkspace> {
    let canonical = canonical_workspace_path(path)?;
    let probe = canonical.join(".loomex-write-test.tmp");
    fs::write(&probe, b"loomex")
        .map_err(|err| CoreError::new("TAURI_WORKSPACE_WRITE_FAILED", err.to_string()))?;
    fs::remove_file(&probe)
        .map_err(|err| CoreError::new("TAURI_WORKSPACE_WRITE_FAILED", err.to_string()))?;
    let display_path = canonical.to_string_lossy().to_string();
    Ok(ValidatedWorkspace {
        fingerprint: stable_fingerprint(&display_path),
        canonical_path: canonical,
        display_path,
    })
}

fn canonical_workspace_path(path: &str) -> CoreResult<PathBuf> {
    let input = PathBuf::from(path);
    if path.trim().is_empty() {
        return Err(CoreError::new(
            "TAURI_WORKSPACE_PATH_REQUIRED",
            "workspace path is required",
        ));
    }
    let metadata = fs::symlink_metadata(&input)
        .map_err(|err| CoreError::new("TAURI_WORKSPACE_PATH_INVALID", err.to_string()))?;
    if metadata.file_type().is_symlink() {
        return Err(CoreError::new(
            "TAURI_WORKSPACE_SYMLINK_NOT_ALLOWED",
            "workspace root cannot be a symlink",
        ));
    }
    if !metadata.is_dir() {
        return Err(CoreError::new(
            "TAURI_WORKSPACE_NOT_DIRECTORY",
            "workspace path must be a directory",
        ));
    }
    let canonical = input
        .canonicalize()
        .map_err(|err| CoreError::new("TAURI_WORKSPACE_PATH_INVALID", err.to_string()))?;
    if canonical.parent().is_none() {
        return Err(CoreError::new(
            "TAURI_WORKSPACE_PATH_UNSAFE",
            "refusing to bind filesystem root",
        ));
    }
    fs::read_dir(&canonical)
        .map_err(|err| CoreError::new("TAURI_WORKSPACE_READ_FAILED", err.to_string()))?
        .next();
    Ok(canonical)
}

fn list_workspace_file_entries(
    root: &Path,
    query: &str,
    limit: usize,
) -> CoreResult<Vec<WorkspaceFileEntry>> {
    let normalized_query = query.trim().trim_start_matches('@').trim().to_lowercase();
    let mut entries = Vec::new();
    collect_workspace_file_entries(root, root, &normalized_query, limit, 0, &mut entries)?;
    entries.sort_by(|a, b| {
        a.is_dir
            .cmp(&b.is_dir)
            .reverse()
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
    entries.truncate(limit);
    Ok(entries)
}

fn run_terminal_command_in_workspace(
    workspace: &Path,
    command: &str,
) -> CoreResult<MacTerminalCommandResponse> {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut child = Command::new(shell)
        .arg("-lc")
        .arg(command)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| CoreError::new("TAURI_TERMINAL_SPAWN_FAILED", err.to_string()))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(err) => {
                return Err(CoreError::new(
                    "TAURI_TERMINAL_WAIT_FAILED",
                    err.to_string(),
                ));
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|err| CoreError::new("TAURI_TERMINAL_OUTPUT_FAILED", err.to_string()))?;
    Ok(MacTerminalCommandResponse {
        schema_version: TERMINAL_COMMAND_SCHEMA.to_string(),
        workspace_path: workspace.to_string_lossy().to_string(),
        command: command.to_string(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        timed_out,
    })
}

fn collect_workspace_file_entries(
    root: &Path,
    current: &Path,
    query: &str,
    limit: usize,
    depth: usize,
    entries: &mut Vec<WorkspaceFileEntry>,
) -> CoreResult<()> {
    if entries.len() >= limit || depth > 6 {
        return Ok(());
    }
    let read_dir = fs::read_dir(current)
        .map_err(|err| CoreError::new("TAURI_WORKSPACE_READ_FAILED", err.to_string()))?;
    for item in read_dir {
        if entries.len() >= limit {
            break;
        }
        let item =
            item.map_err(|err| CoreError::new("TAURI_WORKSPACE_READ_FAILED", err.to_string()))?;
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        if should_skip_workspace_entry(&name) {
            continue;
        }
        let metadata = match item.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let is_dir = metadata.is_dir();
        let relative_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        let searchable = relative_path.to_lowercase();
        if query.is_empty() || searchable.contains(query) {
            entries.push(WorkspaceFileEntry {
                name: name.clone(),
                relative_path: relative_path.clone(),
                path: path.to_string_lossy().to_string(),
                is_dir,
            });
        }
        if is_dir {
            let _ = collect_workspace_file_entries(root, &path, query, limit, depth + 1, entries);
        }
    }
    Ok(())
}

fn should_skip_workspace_entry(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | ".DS_Store"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".next"
            | ".turbo"
            | ".venv"
            | "__pycache__"
    )
}

fn default_runner_capabilities() -> Vec<String> {
    [
        "fs.list",
        "fs.read",
        "fs.write",
        "fs.apply_patch",
        "shell.exec",
        "git.status",
        "git.diff",
        "git.log",
        "http.request",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn local_runner_display_name() -> String {
    env::var("HOSTNAME")
        .or_else(|_| env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "loomex-mac-runner".to_string())
}

fn machine_fingerprint_hash() -> String {
    stable_fingerprint(&format!(
        "{}:{}:{}",
        local_runner_display_name(),
        env::consts::OS,
        env::consts::ARCH
    ))
}

fn stable_fingerprint(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn idempotency_key(prefix: &str, value: &str) -> String {
    format!("{prefix}:{}", stable_fingerprint(value))
}

fn storage_backend_name(backend: CredentialStorageBackend) -> &'static str {
    match backend {
        CredentialStorageBackend::MacOsKeychain => "macos_keychain",
        CredentialStorageBackend::LocalFileFallback => "local_file_fallback",
    }
}

fn workspace_picker_response(path: Option<String>) -> WorkspacePickerResponse {
    WorkspacePickerResponse {
        schema_version: WORKSPACE_PICKER_SCHEMA.to_string(),
        selected: path.is_some(),
        cancelled: path.is_none(),
        path,
    }
}

fn approvals_response_from_inner(inner: &MacAppInner) -> MacApprovalsResponse {
    let approvals = inner
        .approval_order
        .iter()
        .filter_map(|id| {
            inner
                .approval_registry
                .get(id)
                .map(|request| approval_view(request, inner.approval_metadata.get(id)))
        })
        .collect::<Vec<_>>();
    MacApprovalsResponse {
        schema_version: APPROVALS_SCHEMA.to_string(),
        pending_count: approvals
            .iter()
            .filter(|approval| approval.status == "pending")
            .count(),
        approvals,
    }
}

fn approval_view(
    request: &ApprovalRequest,
    metadata: Option<&ApprovalUiMetadata>,
) -> MacApprovalView {
    let prompt = request.prompt();
    let unsupported = metadata
        .map(|metadata| metadata.unsupported_post_mvp)
        .unwrap_or_else(|| !mvp_approval_capability_supported(&prompt.capability));
    let status = if unsupported && request.status == ApprovalStatus::Pending {
        "unsupported_post_mvp".to_string()
    } else {
        approval_status_name(request.status).to_string()
    };
    MacApprovalView {
        approval_request_id: prompt.approval_request_id,
        workflow_run_id: prompt.workflow_run_id,
        node_id: prompt.node_id,
        capability: prompt.capability,
        action_summary: redact_text_for_ui(&prompt.action_summary),
        full_request_details: redact_text_for_ui(&prompt.full_request_details),
        workspace_path: metadata
            .map(|metadata| metadata.workspace_path.clone())
            .unwrap_or_default(),
        risk_summary: if prompt.risk_indicators.is_empty() {
            "low risk".to_string()
        } else {
            prompt.risk_indicators.join(", ")
        },
        risk_level: prompt.risk_level,
        policy_reason: prompt.policy_snapshot.decision_reason,
        status,
        allow_once_enabled: request.status == ApprovalStatus::Pending && !unsupported,
        deny_enabled: request.status == ApprovalStatus::Pending && !unsupported,
        remember_enabled: request.status == ApprovalStatus::Pending
            && !unsupported
            && metadata
                .map(|metadata| metadata.allow_remember)
                .unwrap_or(false),
        unsupported_post_mvp: unsupported,
        expires_at_epoch_ms: prompt.expires_at_epoch_ms,
    }
}

fn pending_approvals_count(inner: &MacAppInner) -> usize {
    inner
        .approval_order
        .iter()
        .filter_map(|id| inner.approval_registry.get(id))
        .filter(|request| request.status == ApprovalStatus::Pending)
        .count()
}

fn validate_approval_context(input: &MacApprovalRequestInput) -> CoreResult<()> {
    for (field, value) in [
        ("approval_request_id", &input.approval_request_id),
        ("workflow_run_id", &input.workflow_run_id),
        ("node_id", &input.node_id),
        ("capability", &input.capability),
        ("action_summary", &input.action_summary),
        ("full_request_details", &input.full_request_details),
        ("workspace_path", &input.workspace_path),
    ] {
        if value.trim().is_empty() {
            return Err(CoreError::new("TAURI_APPROVAL_CONTEXT_INCOMPLETE", field));
        }
    }
    Ok(())
}

fn mvp_approval_capability_supported(capability: &str) -> bool {
    matches!(
        capability,
        "shell.exec"
            | "fs.write"
            | "fs.apply_patch"
            | "git.status"
            | "git.diff"
            | "git.log"
            | "http.request"
    )
}

fn parse_approval_decision(value: &str) -> CoreResult<ApprovalDecision> {
    match value {
        "allow_once" | "allowOnce" => Ok(ApprovalDecision::AllowOnce),
        "deny" => Ok(ApprovalDecision::Deny),
        _ => Err(CoreError::new(
            "TAURI_APPROVAL_DECISION_INVALID",
            "decision must be allow_once or deny",
        )),
    }
}

fn approval_decision_name(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::AllowOnce => "allow_once",
        ApprovalDecision::Deny => "deny",
    }
}

fn approval_status_name(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Expired => "expired",
        ApprovalStatus::Cancelled => "cancelled",
    }
}

fn default_approval_timeout_ms() -> u64 {
    30_000
}

fn default_live_log_limit() -> usize {
    200
}

fn default_workflow_run_list_limit() -> usize {
    12
}

fn default_human_request_action() -> String {
    "submit".to_string()
}

fn default_workspace_file_list_limit() -> usize {
    40
}

fn default_run_history_limit() -> usize {
    25
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn truncate_for_ui(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut output = value.chars().take(limit).collect::<String>();
    output.push_str("...");
    output
}

fn redact_log_entry_for_ui(entry: LogEntry) -> MacLiveLogEntry {
    let mut metadata = entry.metadata;
    redact_json_value_for_ui(&mut metadata);
    MacLiveLogEntry {
        timestamp_epoch_ms: entry.timestamp_epoch_ms,
        level: entry.level,
        event_type: entry.event_type,
        message: redact_text_for_ui(&entry.message),
        workflow_run_id: entry.workflow_run_id,
        tool_call_id: entry.tool_call_id,
        metadata,
    }
}

fn redact_json_value_for_ui(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => *text = redact_text_for_ui(text),
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json_value_for_ui(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if is_sensitive_key(key) {
                    *item = serde_json::Value::String("[REDACTED]".to_string());
                } else {
                    redact_json_value_for_ui(item);
                }
            }
        }
        _ => {}
    }
}

fn redact_text_for_ui(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("bearer ")
        || contains_sensitive_assignment(&lower)
        || contains_cookie_header(&lower)
    {
        "[REDACTED]".to_string()
    } else {
        value.to_string()
    }
}

fn contains_sensitive_assignment(lower: &str) -> bool {
    const SENSITIVE_KEYS: &[&str] = &[
        "authorization",
        "api-key",
        "api_key",
        "api key",
        "apikey",
        "access-token",
        "access_token",
        "refresh-token",
        "refresh_token",
        "token",
        "cookie",
        "set-cookie",
        "credential",
        "password",
        "secret",
    ];
    let bytes = lower.as_bytes();
    for key in SENSITIVE_KEYS {
        let mut search_start = 0;
        while let Some(relative_pos) = lower[search_start..].find(key) {
            let mut index = search_start + relative_pos + key.len();
            while index < bytes.len()
                && matches!(
                    bytes[index],
                    b' ' | b'\t' | b'\r' | b'\n' | b'"' | b'\'' | b'`'
                )
            {
                index += 1;
            }
            if index < bytes.len() && matches!(bytes[index], b':' | b'=') {
                return true;
            }
            search_start += relative_pos + 1;
        }
    }
    false
}

fn contains_cookie_header(lower: &str) -> bool {
    lower.contains("cookie ") || lower.contains("set-cookie ")
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace('-', "_");
    [
        "authorization",
        "api_key",
        "apikey",
        "cookie",
        "credential",
        "password",
        "secret",
        "token",
    ]
    .iter()
    .any(|part| normalized.contains(part))
}

fn active_run_ids_from_logs(path: &Path) -> Vec<String> {
    let entries = loomex_core::read_recent_log_entries(path, 200).unwrap_or_default();
    let mut active = Vec::<String>::new();
    for entry in entries {
        let Some(run_id) = log_entry_run_id(&entry) else {
            continue;
        };
        if matches!(
            entry.event_type.as_str(),
            "workflow.completed" | "workflow.failed" | "workflow.canceled" | "runner.disconnected"
        ) {
            active.retain(|id| id != run_id);
        } else if !active.iter().any(|id| id == run_id) {
            active.push(run_id.to_string());
        }
    }
    active
}

fn run_status_from_event(event_type: &str) -> &'static str {
    match event_type {
        "workflow.completed" => "completed",
        "workflow.failed" => "failed",
        "workflow.canceled" => "canceled",
        _ => "running",
    }
}

fn log_entry_run_id(entry: &LogEntry) -> Option<&str> {
    entry.workflow_run_id.as_deref().or_else(|| {
        if entry.correlation_id.starts_with("run_") {
            Some(entry.correlation_id.as_str())
        } else {
            None
        }
    })
}

fn apply_launch_lifecycle(lifecycle: &mut RunnerStateMachine, resolved: &ResolvedCliSettings) {
    *lifecycle = RunnerStateMachine::new();
    if resolved.organization_id.is_none() {
        return;
    }
    transition_or_reset(lifecycle, RunnerLifecycleEvent::Authenticated);
    if resolved.project_id.is_none() || resolved.binding_id.is_none() {
        transition_or_reset(lifecycle, RunnerLifecycleEvent::ProjectRequired);
        return;
    }
    transition_or_reset(lifecycle, RunnerLifecycleEvent::ProjectBound);
}

fn transition_or_reset(lifecycle: &mut RunnerStateMachine, event: RunnerLifecycleEvent) {
    if lifecycle.transition(event).is_err() {
        *lifecycle = RunnerStateMachine::new();
        let _ = lifecycle.transition(event);
    }
}

fn json_error(error: serde_json::Error) -> CoreError {
    CoreError::new("TAURI_JSON_SERIALIZE_FAILED", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use loomex_core::{
        runner_runtime_guard_path, AuthTokenResponse, CliConfig, HumanRequestResolveResponse,
        HumanRequestSummary, LocalCredentialStore, StreamCredentialRequest,
        StreamCredentialResponse, WorkflowRunStartRequest, WorkflowRunStartResponse,
    };
    use serde_json::Value;
    #[cfg(unix)]
    use std::sync::atomic::{AtomicBool, Ordering};

    #[cfg(unix)]
    struct MockLocalRunnerService {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
        socket_path: PathBuf,
    }

    #[cfg(unix)]
    impl MockLocalRunnerService {
        fn start(home: &Path, binding_id: &str, workspace_path: &Path) -> Self {
            use std::os::unix::{fs::PermissionsExt, net::UnixListener};

            let paths = LocalControlPaths::for_home(home);
            let token = loomex_core::prepare_local_control_paths(&paths).unwrap();
            let listener = UnixListener::bind(&paths.socket_path).unwrap();
            std::fs::set_permissions(&paths.socket_path, std::fs::Permissions::from_mode(0o600))
                .unwrap();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = Arc::clone(&stop);
            let binding_id = binding_id.to_string();
            let workspace_path = workspace_path.to_string_lossy().to_string();
            let thread = std::thread::spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let mut line = String::new();
                            let mut reader = BufReader::new(stream.try_clone().unwrap());
                            if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                                continue;
                            }
                            let request: LocalControlRequest = serde_json::from_str(&line).unwrap();
                            let response = if request.auth_token == token
                                && request.protocol_version == LOCAL_CONTROL_PROTOCOL_VERSION
                            {
                                LocalControlResponse::success(
                                    request.id,
                                    serde_json::json!({
                                        "running": true,
                                        "bindingId": binding_id,
                                        "workspacePath": workspace_path,
                                        "protocolVersion": LOCAL_CONTROL_PROTOCOL_VERSION,
                                    }),
                                )
                            } else {
                                LocalControlResponse::failure(
                                    request.id,
                                    "LOCAL_CONTROL_UNAUTHORIZED",
                                    "invalid local credential",
                                    false,
                                )
                            };
                            serde_json::to_writer(&mut stream, &response).unwrap();
                            stream.write_all(b"\n").unwrap();
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                stop,
                thread: Some(thread),
                socket_path: paths.socket_path,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for MockLocalRunnerService {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = std::os::unix::net::UnixStream::connect(&self.socket_path);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            let _ = std::fs::remove_file(&self.socket_path);
        }
    }

    fn temp_home(label: &str) -> PathBuf {
        // Unix-domain sockets have a small platform path limit (104 bytes on macOS). Keep test
        // homes compact so the production `.loomex/run/control.sock` layout remains testable.
        let mut hasher = DefaultHasher::new();
        label.hash(&mut hasher);
        std::thread::current()
            .name()
            .unwrap_or("test")
            .hash(&mut hasher);
        env::temp_dir().join(format!("lxt-{}-{:x}", std::process::id(), hasher.finish()))
    }

    fn app_for_home(home: &Path) -> MacApp {
        MacApp::new(MacAppPaths::from_home_dir(home)).with_runner_binary(fake_runner_binary(home))
    }

    fn fake_runner_binary(home: &Path) -> PathBuf {
        let bin_dir = home.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        #[cfg(windows)]
        {
            let path = bin_dir.join("loomex-test-runner.cmd");
            std::fs::write(
                &path,
                "@echo off\r\n:loop\r\ntimeout /t 1 /nobreak >nul\r\ngoto loop\r\n",
            )
            .unwrap();
            path
        }
        #[cfg(not(windows))]
        {
            let path = bin_dir.join("loomex-test-runner");
            std::fs::write(
                &path,
                "#!/bin/sh\nset -eu\nguard=\"${LOOMEX_TAURI_GUARD_PATH:-}\"\nbinding=\"${LOOMEX_TAURI_BINDING_ID:-}\"\nif [ -n \"$guard\" ]; then\n  mkdir -p \"$(dirname \"$guard\")\"\n  printf 'surface=loomex-service\\npid=%s\\nbinding_id=%s\\n' \"$$\" \"$binding\" > \"$guard\"\nfi\ncleanup() { [ -z \"$guard\" ] || rm -f \"$guard\"; exit 0; }\ntrap cleanup TERM INT EXIT\nwhile :; do sleep 1; done\n",
            )
            .unwrap();
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).unwrap();
            path
        }
    }

    fn write_bound_cli_config(home: &Path) {
        let path = default_config_path(home);
        let mut config = CliConfig::default();
        config
            .set_key("selectedProfile", "local".to_string())
            .unwrap();
        config
            .set_key("profiles.local.organizationId", "org_123".to_string())
            .unwrap();
        config
            .set_key("profiles.local.projectId", "prj_123".to_string())
            .unwrap();
        config
            .set_key("profiles.local.runnerId", "runner_123".to_string())
            .unwrap();
        config
            .set_key("profiles.local.bindingId", "binding_123".to_string())
            .unwrap();
        let workspace = home.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        config
            .set_key(
                "profiles.local.workspacePath",
                workspace.to_string_lossy().to_string(),
            )
            .unwrap();
        config.save(&path).unwrap();
    }

    fn token(access_token: &str) -> AuthTokenResponse {
        AuthTokenResponse {
            access_token: access_token.to_string(),
            refresh_token: Some("refresh_secret".to_string()),
            token_type: "Bearer".to_string(),
            expires_at: "2026-06-29T00:00:00Z".to_string(),
        }
    }

    fn credential(profile: &str, organization_id: &str) -> ManagementCredential {
        ManagementCredential::from_token_response(
            profile,
            organization_id,
            token("management_secret"),
            CredentialStorageBackend::LocalFileFallback,
        )
        .unwrap()
    }

    fn approval_input(id: &str, capability: &str) -> MacApprovalRequestInput {
        MacApprovalRequestInput {
            approval_request_id: id.to_string(),
            workflow_run_id: "run_123".to_string(),
            node_id: "node_123".to_string(),
            capability: capability.to_string(),
            action_summary: format!("Approve {capability}"),
            full_request_details: format!("{capability} in /Users/test/workspace"),
            risk_indicators: vec!["local_action".to_string()],
            workspace_path: "/Users/test/workspace".to_string(),
            allow_remember: false,
            policy_reason: "ask by policy".to_string(),
            timeout_ms: 30_000,
            now_epoch_ms: Some(1_000),
            authorized_user_ids: vec!["mac-user".to_string()],
        }
    }

    #[derive(Default)]
    struct FakeManagementClient {
        device_challenge: Option<DeviceLoginChallenge>,
        device_token: Option<AuthTokenResponse>,
        api_key_token: Option<AuthTokenResponse>,
        api_key_error: Option<CoreError>,
        workspace_login: Option<loomex_core::WorkspaceLoginResult>,
        workspace_login_error: Option<CoreError>,
        workspace_bootstrap_token: Option<AuthTokenResponse>,
        organizations: Vec<Organization>,
        projects: Vec<Project>,
        project: Option<Project>,
        runner: Option<loomex_core::Runner>,
        binding: Option<ManagementProjectRunnerBinding>,
        binding_error: Option<CoreError>,
        workflows: Vec<RunnerWorkflowSummary>,
        runner_workflow_input_schema: Option<Value>,
        runner_workflow_active_version: Option<Value>,
        runner_workflow_selected_version: Option<Value>,
        runner_workflow_versions: Vec<Value>,
        runner_execution_list_response: Option<RunnerWorkflowExecutionListResponse>,
        last_runner_workflow_id: Option<String>,
        last_runner_workflow_inputs: Option<Value>,
        last_runner_workflow_execution_version: Option<String>,
        last_runner_workflow_schema_version: Option<String>,
        last_runner_execution_mode: Option<String>,
        runner_execution_response: Option<RunnerWorkflowExecutionResponse>,
        runner_execution_detail_response: Option<RunnerWorkflowExecutionResponse>,
        human_requests: Vec<HumanRequestSummary>,
        last_human_request_workflow_id: Option<String>,
        last_human_request_execution_id: Option<String>,
        last_human_request_id: Option<String>,
        last_human_request_payload: Option<Value>,
        human_request_resolve_response: Option<HumanRequestResolveResponse>,
    }

    impl ManagementApiClient for FakeManagementClient {
        fn start_device_login(&mut self) -> CoreResult<DeviceLoginChallenge> {
            self.device_challenge
                .clone()
                .ok_or_else(|| CoreError::new("DEVICE_LOGIN_UNAVAILABLE", "no challenge"))
        }

        fn poll_device_token(
            &mut self,
            _device_code: &str,
        ) -> CoreResult<Option<AuthTokenResponse>> {
            Ok(self.device_token.take())
        }

        fn exchange_api_key(
            &mut self,
            _api_key: &str,
            _api_secret: &str,
            _organization_id: &str,
        ) -> CoreResult<ApiKeyExchangeResult> {
            if let Some(error) = self.api_key_error.clone() {
                return Err(error);
            }
            self.api_key_token
                .clone()
                .map(|token| {
                    let mut exchange = ApiKeyExchangeResult::from_token(token);
                    exchange.organization_id = Some("org_123".to_string());
                    exchange
                })
                .ok_or_else(|| CoreError::new("MANAGEMENT_AUTH_FAILED", "invalid API key"))
        }

        fn login_workspace(
            &mut self,
            _email: &str,
            _password: &str,
        ) -> CoreResult<loomex_core::WorkspaceLoginResult> {
            if let Some(error) = self.workspace_login_error.clone() {
                return Err(error);
            }
            self.workspace_login.clone().ok_or_else(|| {
                CoreError::new("MANAGEMENT_AUTH_FAILED", "invalid workspace credentials")
            })
        }

        fn bootstrap_runner_with_workspace_token(
            &mut self,
            _workspace_token: &str,
            organization_id: &str,
            project_id: Option<&str>,
            _workspace_root: Option<&str>,
        ) -> CoreResult<ApiKeyExchangeResult> {
            self.workspace_bootstrap_token
                .clone()
                .map(|token| {
                    let mut exchange = ApiKeyExchangeResult::from_token(token);
                    exchange.organization_id = Some(organization_id.to_string());
                    exchange.project_id = project_id.map(ToString::to_string);
                    exchange.runner_id = Some("runner_123".to_string());
                    exchange.binding_id = project_id.map(|_| "runner_123".to_string());
                    exchange
                })
                .ok_or_else(|| CoreError::new("MANAGEMENT_AUTH_FAILED", "runner bootstrap failed"))
        }

        fn list_organizations(
            &mut self,
            _credential: &ManagementCredential,
        ) -> CoreResult<Vec<Organization>> {
            Ok(self.organizations.clone())
        }

        fn list_projects(
            &mut self,
            _credential: &ManagementCredential,
            _organization_id: &str,
        ) -> CoreResult<Vec<Project>> {
            Ok(self.projects.clone())
        }

        fn get_project(
            &mut self,
            _credential: &ManagementCredential,
            project_id: &str,
        ) -> CoreResult<Project> {
            self.project.clone().ok_or_else(|| {
                CoreError::new(
                    "PROJECT_NOT_FOUND",
                    format!("project {project_id} is unavailable"),
                )
            })
        }

        fn get_current_runner(
            &mut self,
            _credential: &ManagementCredential,
            _organization_id: &str,
        ) -> CoreResult<loomex_core::Runner> {
            self.runner
                .clone()
                .ok_or_else(|| CoreError::new("RUNNER_NOT_FOUND", "runner unavailable"))
        }

        fn upsert_current_runner(
            &mut self,
            _credential: &ManagementCredential,
            request: &RunnerUpsertRequest,
            _idempotency_key: &str,
        ) -> CoreResult<loomex_core::Runner> {
            Ok(loomex_core::Runner {
                id: "runner_123".to_string(),
                organization_id: request.organization_id.clone(),
                status: "connected".to_string(),
                runner_version: request.runner_version.clone(),
                protocol_version: request.protocol_version.clone(),
                capabilities: request.capabilities.clone(),
            })
        }

        fn create_project_runner_binding(
            &mut self,
            _credential: &ManagementCredential,
            project_id: &str,
            request: &ProjectRunnerBindingCreateRequest,
            _idempotency_key: &str,
        ) -> CoreResult<ManagementProjectRunnerBinding> {
            if let Some(error) = self.binding_error.clone() {
                return Err(error);
            }
            Ok(self
                .binding
                .clone()
                .unwrap_or_else(|| ManagementProjectRunnerBinding {
                    id: "binding_123".to_string(),
                    organization_id: request.organization_id.clone(),
                    project_id: project_id.to_string(),
                    runner_id: request.runner_id.clone(),
                    local_root_path: request.local_root_path.clone(),
                    status: "active".to_string(),
                    local_root_fingerprint: request.local_root_fingerprint.clone(),
                }))
        }

        fn list_project_runner_bindings(
            &mut self,
            _credential: &ManagementCredential,
            _project_id: &str,
        ) -> CoreResult<Vec<ManagementProjectRunnerBinding>> {
            Ok(self.binding.clone().into_iter().collect())
        }

        fn revoke_project_runner_binding(
            &mut self,
            _credential: &ManagementCredential,
            _project_id: &str,
            _binding_id: &str,
            _idempotency_key: &str,
        ) -> CoreResult<()> {
            Ok(())
        }

        fn start_workflow_run(
            &mut self,
            _credential: &ManagementCredential,
            _request: &WorkflowRunStartRequest,
        ) -> CoreResult<WorkflowRunStartResponse> {
            Err(CoreError::new("TEST_UNIMPLEMENTED", "unused"))
        }

        fn list_runner_workflows(
            &mut self,
            _credential: &ManagementCredential,
        ) -> CoreResult<Vec<RunnerWorkflowSummary>> {
            Ok(self.workflows.clone())
        }

        fn start_runner_workflow_execution(
            &mut self,
            _credential: &ManagementCredential,
            workflow_id: &str,
            inputs: Value,
            _session_id: Option<&str>,
            version: Option<&str>,
        ) -> CoreResult<RunnerWorkflowExecutionResponse> {
            self.last_runner_workflow_id = Some(workflow_id.to_string());
            self.last_runner_workflow_inputs = Some(inputs);
            self.last_runner_workflow_execution_version = version.map(str::to_string);
            Ok(self.runner_execution_response.clone().unwrap_or_else(|| {
                RunnerWorkflowExecutionResponse {
                    execution: serde_json::json!({
                        "id": "exec_123",
                        "status": "queued"
                    }),
                    human_request: None,
                    runner: Some(serde_json::json!({
                        "id": "runner_123"
                    })),
                    events: Vec::new(),
                    ai_trace: None,
                    latest_sequence: 0,
                    timed_out: false,
                    extra: serde_json::Map::new(),
                }
            }))
        }

        fn start_runner_workflow_execution_scoped(
            &mut self,
            credential: &ManagementCredential,
            options: RunnerWorkflowExecutionStartOptions<'_>,
        ) -> CoreResult<RunnerWorkflowExecutionResponse> {
            self.last_runner_execution_mode = options.execution_mode.map(str::to_string);
            self.start_runner_workflow_execution(
                credential,
                options.workflow_id,
                options.inputs,
                options.session_id,
                options.version,
            )
        }

        fn get_runner_workflow_execution(
            &mut self,
            _credential: &ManagementCredential,
            _execution_id: &str,
        ) -> CoreResult<RunnerWorkflowExecutionResponse> {
            Ok(self
                .runner_execution_detail_response
                .clone()
                .unwrap_or_else(|| RunnerWorkflowExecutionResponse {
                    execution: serde_json::json!({
                        "id": "exec_123",
                        "status": "succeeded",
                        "output": {
                            "summary": {
                                "text": "Workflow completed."
                            }
                        }
                    }),
                    human_request: None,
                    runner: None,
                    events: Vec::new(),
                    ai_trace: None,
                    latest_sequence: 0,
                    timed_out: false,
                    extra: serde_json::Map::new(),
                }))
        }

        fn get_runner_workflow_execution_scoped(
            &mut self,
            credential: &ManagementCredential,
            execution_id: &str,
            execution_mode: Option<&str>,
        ) -> CoreResult<RunnerWorkflowExecutionResponse> {
            self.last_runner_execution_mode = execution_mode.map(str::to_string);
            self.get_runner_workflow_execution(credential, execution_id)
        }

        fn list_runner_workflow_executions(
            &mut self,
            _credential: &ManagementCredential,
            _workflow_id: &str,
            _limit: usize,
        ) -> CoreResult<RunnerWorkflowExecutionListResponse> {
            Ok(self
                .runner_execution_list_response
                .clone()
                .unwrap_or_else(|| RunnerWorkflowExecutionListResponse {
                    executions: vec![serde_json::json!({
                        "id": "exec_123",
                        "workflowId": "workflow_123",
                        "status": "succeeded",
                        "input": {
                            "prompt": "Run the workflow"
                        },
                        "output": {
                            "summary": {
                                "text": "Workflow completed."
                            }
                        }
                    })],
                    next_cursor: None,
                }))
        }

        fn list_runner_workflow_executions_filtered_scoped(
            &mut self,
            credential: &ManagementCredential,
            workflow_id: &str,
            execution_mode: Option<&str>,
            _status: Option<&str>,
            _cursor: Option<&str>,
            limit: usize,
        ) -> CoreResult<RunnerWorkflowExecutionListResponse> {
            self.last_runner_execution_mode = execution_mode.map(str::to_string);
            self.list_runner_workflow_executions(credential, workflow_id, limit)
        }

        fn get_runner_workflow_input_schema(
            &mut self,
            _credential: &ManagementCredential,
            _workflow_id: &str,
            version: Option<&str>,
        ) -> CoreResult<loomex_core::RunnerWorkflowInputSchemaResponse> {
            self.last_runner_workflow_schema_version = version.map(str::to_string);
            Ok(loomex_core::RunnerWorkflowInputSchemaResponse {
                workflow: None,
                input_schema: self.runner_workflow_input_schema.clone(),
                active_version: self.runner_workflow_active_version.clone(),
                selected_version: self.runner_workflow_selected_version.clone(),
                versions: self.runner_workflow_versions.clone(),
                first_human_input: None,
                nodes: Vec::new(),
                capabilities: serde_json::Map::new(),
                extra: serde_json::Map::new(),
            })
        }

        fn get_runner_workflow_input_schema_scoped(
            &mut self,
            credential: &ManagementCredential,
            workflow_id: &str,
            version: Option<&str>,
            execution_mode: Option<&str>,
        ) -> CoreResult<loomex_core::RunnerWorkflowInputSchemaResponse> {
            self.last_runner_execution_mode = execution_mode.map(str::to_string);
            self.get_runner_workflow_input_schema(credential, workflow_id, version)
        }

        fn get_workflow_input_schema(
            &mut self,
            _credential: &ManagementCredential,
            _workflow_id: &str,
        ) -> CoreResult<Option<Value>> {
            Ok(None)
        }

        fn list_human_requests(
            &mut self,
            _credential: &ManagementCredential,
            workflow_id: &str,
            execution_id: Option<&str>,
        ) -> CoreResult<Vec<HumanRequestSummary>> {
            self.last_human_request_workflow_id = Some(workflow_id.to_string());
            self.last_human_request_execution_id = execution_id.map(str::to_string);
            Ok(self.human_requests.clone())
        }

        fn resolve_human_request(
            &mut self,
            _credential: &ManagementCredential,
            request_id: &str,
            payload: &Value,
        ) -> CoreResult<HumanRequestResolveResponse> {
            self.last_human_request_id = Some(request_id.to_string());
            self.last_human_request_payload = Some(payload.clone());
            Ok(self
                .human_request_resolve_response
                .clone()
                .unwrap_or_else(|| HumanRequestResolveResponse {
                    request_id: request_id.to_string(),
                    request_status: "resolved".to_string(),
                    execution_id: Some("exec_123".to_string()),
                    execution_status: Some("running".to_string()),
                }))
        }

        fn create_runner_session(
            &mut self,
            _credential: &ManagementCredential,
            _workspace_root: &str,
            _manifest: Value,
            _transport: &str,
        ) -> CoreResult<loomex_core::RunnerSessionResponse> {
            Err(CoreError::new("TEST_UNIMPLEMENTED", "unused"))
        }

        fn heartbeat_runner_session(
            &mut self,
            _credential: &ManagementCredential,
            _session_id: &str,
            _manifest: Value,
        ) -> CoreResult<loomex_core::RunnerSessionResponse> {
            Err(CoreError::new("TEST_UNIMPLEMENTED", "unused"))
        }

        fn lease_runner_job(
            &mut self,
            _credential: &ManagementCredential,
            _session_id: &str,
        ) -> CoreResult<loomex_core::RunnerJobResponse> {
            Err(CoreError::new("TEST_UNIMPLEMENTED", "unused"))
        }

        fn start_runner_job(
            &mut self,
            _credential: &ManagementCredential,
            _session_id: &str,
            _job_id: &str,
        ) -> CoreResult<loomex_core::RunnerJobResponse> {
            Err(CoreError::new("TEST_UNIMPLEMENTED", "unused"))
        }

        fn append_runner_job_events(
            &mut self,
            _credential: &ManagementCredential,
            _session_id: &str,
            _job_id: &str,
            _events: Vec<Value>,
        ) -> CoreResult<loomex_core::RunnerJobEventCreateResponse> {
            Err(CoreError::new("TEST_UNIMPLEMENTED", "unused"))
        }

        fn complete_runner_job(
            &mut self,
            _credential: &ManagementCredential,
            _session_id: &str,
            _job_id: &str,
            _result: Value,
        ) -> CoreResult<loomex_core::RunnerJobResponse> {
            Err(CoreError::new("TEST_UNIMPLEMENTED", "unused"))
        }

        fn fail_runner_job(
            &mut self,
            _credential: &ManagementCredential,
            _session_id: &str,
            _job_id: &str,
            _error: Value,
        ) -> CoreResult<loomex_core::RunnerJobResponse> {
            Err(CoreError::new("TEST_UNIMPLEMENTED", "unused"))
        }

        fn issue_stream_credential(
            &mut self,
            _credential: &ManagementCredential,
            _request: &StreamCredentialRequest,
            _idempotency_key: &str,
        ) -> CoreResult<StreamCredentialResponse> {
            Err(CoreError::new("TEST_UNIMPLEMENTED", "unused"))
        }
    }

    #[test]
    fn tauri_command_smoke_test() {
        let home = temp_home("command-smoke");
        let app = app_for_home(&home);

        let status = tauri_app_launch(&app, AppLaunchRequest { profile: None }).unwrap();
        let encoded = serde_json::to_value(status).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(APP_SURFACE_NAME, app_surface_name());
        assert_eq!(APP_STATUS_SCHEMA, encoded["schemaVersion"]);
        assert_eq!("not_authenticated", encoded["lifecycle"]);
    }

    #[test]
    fn app_launch_without_config_uses_cli_default_path_and_defaults() {
        let home = temp_home("no-config");
        let app = app_for_home(&home);

        let status = app.launch(AppLaunchRequest { profile: None }).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("local", status.profile);
        assert_eq!("http://127.0.0.1:28000", status.server_url);
        assert_eq!("not_authenticated", status.lifecycle);
        assert!(status.config_path.ends_with(".loomex/config.toml"));
    }

    #[test]
    fn api_key_login_success_persists_credential_and_org_context() {
        let home = temp_home("api-login-success");
        let app = app_for_home(&home);
        let mut config = CliConfig::default();
        let store = LocalCredentialStore::new(home.join(".loomex").join("credentials"));
        let mut client = FakeManagementClient {
            api_key_token: Some(token("management_secret")),
            ..Default::default()
        };

        let response = app
            .login_api_key_with(
                ApiKeyLoginRequest {
                    profile: None,
                    api_key: "wfpk_123".to_string(),
                    api_secret: "wfsk_123".to_string(),
                    organization_id: None,
                },
                &mut config,
                &store,
                &mut client,
                CredentialStorageBackend::LocalFileFallback,
            )
            .unwrap();
        let saved = store.load("local").unwrap().unwrap();
        let saved_config = CliConfig::load_or_default(&default_config_path(&home)).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert!(response.authenticated);
        assert_eq!("org_123", response.organization_id);
        assert_eq!("management_secret", saved.access_token);
        assert_eq!(
            Some("org_123".to_string()),
            saved_config.profiles["local"].organization_id
        );
    }

    #[test]
    fn workspace_login_bootstraps_runner_credential_and_context() {
        let home = temp_home("workspace-login-success");
        let app = app_for_home(&home);
        let mut config = CliConfig::default();
        let store = LocalCredentialStore::new(home.join(".loomex").join("credentials"));
        let mut client = FakeManagementClient {
            workspace_login: Some(loomex_core::WorkspaceLoginResult {
                token: "workspace_session".to_string(),
                organization_id: Some("org_123".to_string()),
                project_id: Some("project_123".to_string()),
            }),
            workspace_bootstrap_token: Some(token("runner_secret")),
            ..Default::default()
        };

        let response = app
            .login_workspace_with(
                WorkspaceLoginRequest {
                    profile: None,
                    email: "admin@example.com".to_string(),
                    password: "change-me".to_string(),
                },
                &mut config,
                &store,
                &mut client,
                CredentialStorageBackend::LocalFileFallback,
                Some("/tmp/workspace"),
            )
            .unwrap();
        let saved = store.load("local").unwrap().unwrap();
        let saved_config = CliConfig::load_or_default(&default_config_path(&home)).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert!(response.authenticated);
        assert_eq!("org_123", response.organization_id);
        assert_eq!("runner_secret", saved.access_token);
        assert_eq!(
            Some("project_123".to_string()),
            saved_config.profiles["local"].project_id
        );
    }

    #[test]
    fn device_login_cancel_does_not_authenticate_or_mutate_state() {
        let home = temp_home("device-cancel");
        let app = app_for_home(&home);

        let status = app.cancel_device_login(None).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert!(!status.authenticated);
        assert_eq!("not_authenticated", status.lifecycle);
    }

    #[test]
    fn device_login_stores_user_token_separately_until_project_bootstrap() {
        let home = temp_home("device-login-user-token");
        let app = app_for_home(&home);
        let mut config = CliConfig::default();
        let store = LocalCredentialStore::new(home.join(".loomex").join("credentials"));
        let mut client = FakeManagementClient {
            device_token: Some(token("signed-user-token")),
            organizations: vec![Organization {
                id: "org_123".to_string(),
                name: "Only Org".to_string(),
            }],
            ..Default::default()
        };

        let response = app
            .complete_device_login_with(
                DeviceLoginCompleteRequest {
                    profile: None,
                    device_code: "device-1".to_string(),
                    organization_id: None,
                },
                &mut config,
                &store,
                &mut client,
                CredentialStorageBackend::LocalFileFallback,
            )
            .unwrap();

        assert!(response.authenticated);
        assert_eq!(
            "signed-user-token",
            store.load("local.user").unwrap().unwrap().access_token
        );
        assert!(store.load("local").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn project_selection_bootstraps_and_stores_runner_token() {
        let home = temp_home("device-project-bootstrap");
        let app = app_for_home(&home);
        let mut config = CliConfig::default();
        let store = LocalCredentialStore::new(home.join(".loomex").join("credentials"));
        let user_credential = ManagementCredential::from_user_token_response(
            "local.user",
            "org_123",
            token("signed-user-token"),
            CredentialStorageBackend::LocalFileFallback,
        )
        .unwrap();
        store.save(&user_credential).unwrap();
        let mut client = FakeManagementClient {
            project: Some(Project {
                id: "project_123".to_string(),
                organization_id: "org_123".to_string(),
                name: "Project".to_string(),
                status: "active".to_string(),
            }),
            workspace_bootstrap_token: Some(token("lmxrt_runner-token")),
            ..Default::default()
        };

        app.select_project_with(
            ProjectSelectRequest {
                profile: None,
                organization_id: "org_123".to_string(),
                project_id: "project_123".to_string(),
            },
            &mut config,
            &store,
            &user_credential,
            &mut client,
        )
        .unwrap();

        assert_eq!(
            "lmxrt_runner-token",
            store.load("local").unwrap().unwrap().access_token
        );
        assert_eq!(
            Some("runner_123".to_string()),
            config.profiles["local"].runner_id
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn api_key_login_invalid_returns_structured_error() {
        let home = temp_home("api-login-invalid");
        let app = app_for_home(&home);
        let mut config = CliConfig::default();
        let store = LocalCredentialStore::new(home.join(".loomex").join("credentials"));
        let mut client = FakeManagementClient {
            api_key_error: Some(CoreError::new("MANAGEMENT_AUTH_FAILED", "invalid")),
            ..Default::default()
        };

        let err = app
            .login_api_key_with(
                ApiKeyLoginRequest {
                    profile: None,
                    api_key: "wfpk_bad".to_string(),
                    api_secret: "wfsk_bad".to_string(),
                    organization_id: None,
                },
                &mut config,
                &store,
                &mut client,
                CredentialStorageBackend::LocalFileFallback,
            )
            .unwrap_err();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("MANAGEMENT_AUTH_FAILED", err.code);
    }

    #[test]
    fn project_list_empty_returns_actionable_error() {
        let home = temp_home("project-empty");
        let app = app_for_home(&home);
        let mut client = FakeManagementClient::default();
        let credential = credential("default", "org_empty");

        let err = app
            .list_projects_with(&credential, &mut client, "org_empty")
            .unwrap_err();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("TAURI_PROJECT_ACCESS_EMPTY", err.code);
    }

    #[test]
    fn workspace_picker_cancel_preserves_selected_state() {
        let home = temp_home("workspace-cancel");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);

        let before = app.status(None).unwrap();
        let after = app
            .workspace_picker_cancel(WorkspacePickerCancelRequest { profile: None })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(before.binding_id, after.binding_id);
        assert_eq!(before.workspace_path, after.workspace_path);
    }

    #[test]
    fn native_workspace_picker_response_exposes_selected_path_and_cancel() {
        let selected = workspace_picker_response(Some("/Users/test/repo".to_string()));
        let cancelled = workspace_picker_response(None);

        assert_eq!(WORKSPACE_PICKER_SCHEMA, selected.schema_version);
        assert!(selected.selected);
        assert!(!selected.cancelled);
        assert_eq!(Some("/Users/test/repo".to_string()), selected.path);
        assert!(!cancelled.selected);
        assert!(cancelled.cancelled);
        assert_eq!(None, cancelled.path);
    }

    #[test]
    fn workspace_set_persists_local_path_without_project_binding() {
        let home = temp_home("workspace-set");
        let workspace = home.join("repo");
        std::fs::create_dir_all(&workspace).unwrap();
        let app = app_for_home(&home);
        let mut config = CliConfig::default();

        let response = app
            .set_workspace_with(
                WorkspaceSetRequest {
                    profile: None,
                    workspace_path: workspace.to_string_lossy().to_string(),
                },
                &mut config,
            )
            .unwrap();
        let saved = CliConfig::load_or_default(&default_config_path(&home)).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(WORKSPACE_SET_SCHEMA, response.schema_version);
        assert!(response.workspace_path.ends_with("repo"));
        assert_eq!(None, saved.profiles["local"].binding_id);
        assert!(saved.profiles["local"]
            .workspace_path
            .as_deref()
            .unwrap()
            .ends_with("repo"));
    }

    #[test]
    fn workspace_file_list_returns_mentions_from_selected_workspace() {
        let home = temp_home("workspace-file-list");
        let workspace = home.join("repo");
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(workspace.join("src").join("main.rs"), b"fn main() {}").unwrap();
        let app = app_for_home(&home);
        let mut config = CliConfig::default();
        app.set_workspace_with(
            WorkspaceSetRequest {
                profile: Some("default".to_string()),
                workspace_path: workspace.to_string_lossy().to_string(),
            },
            &mut config,
        )
        .unwrap();

        let response = app
            .list_workspace_files(WorkspaceFileListRequest {
                profile: Some("default".to_string()),
                query: Some("src".to_string()),
                limit: 10,
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(WORKSPACE_FILE_LIST_SCHEMA, response.schema_version);
        assert!(response
            .entries
            .iter()
            .any(|entry| entry.relative_path == "src" && entry.is_dir));
        assert!(response
            .entries
            .iter()
            .any(|entry| entry.relative_path == "src/main.rs" && !entry.is_dir));
    }

    #[test]
    fn workspace_file_read_returns_content_from_selected_workspace() {
        let home = temp_home("workspace-file-read");
        let workspace = home.join("repo");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("README.md"), b"# Hello\nWorld").unwrap();
        let app = app_for_home(&home);
        let mut config = CliConfig::default();
        app.set_workspace_with(
            WorkspaceSetRequest {
                profile: Some("default".to_string()),
                workspace_path: workspace.to_string_lossy().to_string(),
            },
            &mut config,
        )
        .unwrap();

        let response = app
            .read_workspace_file(WorkspaceFileReadRequest {
                profile: Some("default".to_string()),
                path: "README.md".to_string(),
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(WORKSPACE_FILE_READ_SCHEMA, response.schema_version);
        assert_eq!("README.md", response.relative_path);
        assert_eq!("# Hello\nWorld", response.content);
    }

    #[test]
    fn terminal_command_runs_inside_selected_workspace() {
        let home = temp_home("terminal-command");
        let workspace = home.join("repo");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("marker.txt"), b"ready").unwrap();
        let app = app_for_home(&home);
        let mut config = CliConfig::default();
        app.set_workspace_with(
            WorkspaceSetRequest {
                profile: Some("default".to_string()),
                workspace_path: workspace.to_string_lossy().to_string(),
            },
            &mut config,
        )
        .unwrap();

        let response = app
            .run_terminal_command(TerminalCommandRequest {
                profile: Some("default".to_string()),
                command: "pwd; cat marker.txt".to_string(),
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(TERMINAL_COMMAND_SCHEMA, response.schema_version);
        assert_eq!(Some(0), response.exit_code);
        assert!(!response.timed_out);
        assert!(response.stdout.contains("repo"));
        assert!(response.stdout.contains("ready"));
    }

    #[test]
    fn workflow_list_uses_runner_control_workflows() {
        let home = temp_home("workflow-list");
        let app = app_for_home(&home);
        let credential = credential("default", "org_123");
        let mut client = FakeManagementClient {
            workflows: vec![RunnerWorkflowSummary {
                id: "wf_123".to_string(),
                name: "Code review".to_string(),
                title: None,
                description: Some("Review current changes".to_string()),
                status: Some("active".to_string()),
                active_version_id: None,
                extra: serde_json::Map::new(),
            }],
            ..Default::default()
        };

        let response = app.list_workflows_with(&credential, &mut client).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(WORKFLOW_LIST_SCHEMA, response.schema_version);
        assert_eq!("wf_123", response.workflows[0].id);
    }

    #[test]
    fn workflow_run_list_uses_runner_control_execution_history() {
        let home = temp_home("workflow-run-list");
        let app = app_for_home(&home);
        let credential = credential("default", "org_123");
        let mut client = FakeManagementClient {
            runner_execution_list_response: Some(RunnerWorkflowExecutionListResponse {
                executions: vec![serde_json::json!({
                    "id": "exec_456",
                    "workflowId": "wf_123",
                    "status": "failed",
                    "input": {
                        "prompt": "Fix backend"
                    }
                })],
                next_cursor: None,
            }),
            ..Default::default()
        };

        let response = app
            .list_workflow_runs_with("wf_123".to_string(), 5, &credential, &mut client)
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(WORKFLOW_RUN_LIST_SCHEMA, response.schema_version);
        assert_eq!("wf_123", response.workflow_id);
        assert_eq!("exec_456", response.result.executions[0]["id"]);
        assert_eq!(Some("app".to_string()), client.last_runner_execution_mode);
    }

    #[test]
    fn workflow_input_schema_uses_runner_control_detail() {
        let home = temp_home("workflow-input-schema");
        let app = app_for_home(&home);
        let credential = credential("default", "org_123");
        let mut client = FakeManagementClient {
            runner_workflow_input_schema: Some(serde_json::json!({
                "type": "object",
                "required": ["prompt"],
                "properties": {
                    "prompt": { "type": "string" }
                }
            })),
            runner_workflow_active_version: Some(serde_json::json!({
                "versionNumber": 1
            })),
            runner_workflow_selected_version: Some(serde_json::json!({
                "versionNumber": 2
            })),
            runner_workflow_versions: vec![
                serde_json::json!({ "versionNumber": 2, "status": "archived" }),
                serde_json::json!({ "versionNumber": 1, "status": "published", "isActive": true }),
            ],
            ..Default::default()
        };

        let response = app
            .workflow_input_schema_with(
                "wf_123".to_string(),
                Some("2".to_string()),
                &credential,
                &mut client,
            )
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(WORKFLOW_INPUT_SCHEMA_SCHEMA, response.schema_version);
        assert_eq!("wf_123", response.workflow_id);
        assert_eq!(
            Some("2".to_string()),
            client.last_runner_workflow_schema_version
        );
        assert_eq!(Some("app".to_string()), client.last_runner_execution_mode);
        assert_eq!(
            2,
            response.selected_version.as_ref().unwrap()["versionNumber"]
        );
        assert_eq!(2, response.versions.len());
        assert_eq!(
            Some("prompt"),
            response
                .input_schema
                .as_ref()
                .and_then(|schema| schema["required"].as_array())
                .and_then(|required| required.first())
                .and_then(|value| value.as_str())
        );
    }

    #[test]
    fn workflow_run_chat_sends_prompt_and_workspace_to_runner_control() {
        let home = temp_home("workflow-run-chat");
        let app = app_for_home(&home);
        let credential = credential("default", "org_123");
        let mut client = FakeManagementClient::default();

        let response = app
            .run_workflow_chat_with(
                WorkflowRunChatRequest {
                    profile: None,
                    workflow_id: "wf_123".to_string(),
                    version: None,
                    inputs: serde_json::Map::new(),
                    prompt: "Summarize this repo".to_string(),
                    selected_files: vec!["/Users/test/repo/src".to_string()],
                    workspace_path: Some("/Users/test/repo".to_string()),
                    session_id: None,
                },
                None,
                Some("binding_123"),
                &credential,
                &mut client,
            )
            .unwrap();
        let inputs = client.last_runner_workflow_inputs.unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(WORKFLOW_RUN_CHAT_SCHEMA, response.schema_version);
        assert_eq!(Some("wf_123".to_string()), client.last_runner_workflow_id);
        assert_eq!(Some("app".to_string()), client.last_runner_execution_mode);
        assert_eq!("Summarize this repo", inputs["prompt"]);
        assert_eq!("Summarize this repo", inputs["message"]);
        assert_eq!("/Users/test/repo", inputs["workspacePath"]);
        assert_eq!("/Users/test/repo/src", inputs["selectedFiles"][0]);
        assert_eq!("exec_123", response.result.execution["id"]);
    }

    #[test]
    fn workflow_run_chat_allows_empty_prompt_without_default_input() {
        let home = temp_home("workflow-run-empty-chat");
        let app = app_for_home(&home);
        let credential = credential("default", "org_123");
        let mut client = FakeManagementClient::default();

        let response = app
            .run_workflow_chat_with(
                WorkflowRunChatRequest {
                    profile: None,
                    workflow_id: "wf_123".to_string(),
                    version: None,
                    inputs: serde_json::Map::new(),
                    prompt: "".to_string(),
                    selected_files: Vec::new(),
                    workspace_path: Some("/Users/test/repo".to_string()),
                    session_id: None,
                },
                None,
                Some("binding_123"),
                &credential,
                &mut client,
            )
            .unwrap();
        let inputs = client.last_runner_workflow_inputs.unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(WORKFLOW_RUN_CHAT_SCHEMA, response.schema_version);
        assert!(inputs.get("prompt").is_none());
        assert!(inputs.get("message").is_none());
        assert_eq!("/Users/test/repo", inputs["workspacePath"]);
    }

    #[test]
    fn workflow_run_chat_sends_explicit_schema_inputs_to_runner_control() {
        let home = temp_home("workflow-run-explicit-inputs");
        let app = app_for_home(&home);
        let credential = credential("default", "org_123");
        let mut client = FakeManagementClient::default();
        let mut explicit_inputs = serde_json::Map::new();
        explicit_inputs.insert("ticketId".to_string(), serde_json::json!(42));
        explicit_inputs.insert(
            "prompt".to_string(),
            serde_json::Value::String("Build the audit endpoint".to_string()),
        );

        app.run_workflow_chat_with(
            WorkflowRunChatRequest {
                profile: None,
                workflow_id: "wf_123".to_string(),
                version: Some("2".to_string()),
                inputs: explicit_inputs,
                prompt: "".to_string(),
                selected_files: Vec::new(),
                workspace_path: Some("/Users/test/repo".to_string()),
                session_id: None,
            },
            None,
            Some("binding_123"),
            &credential,
            &mut client,
        )
        .unwrap();
        let inputs = client.last_runner_workflow_inputs.unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("Build the audit endpoint", inputs["prompt"]);
        assert_eq!(42, inputs["ticketId"]);
        assert_eq!("/Users/test/repo", inputs["workspacePath"]);
        assert!(inputs.get("message").is_none());
        assert_eq!(
            Some("2".to_string()),
            client.last_runner_workflow_execution_version
        );
    }

    #[test]
    fn human_request_list_uses_workflow_and_execution_scope() {
        let home = temp_home("human-request-list");
        let app = app_for_home(&home);
        let credential = credential("default", "org_123");
        let mut client = FakeManagementClient {
            human_requests: vec![HumanRequestSummary {
                id: "hr_123".to_string(),
                status: "pending".to_string(),
                title: "Review output".to_string(),
                execution: Some(loomex_core::HumanRequestExecution {
                    id: "exec_123".to_string(),
                }),
                description: "Provide review notes".to_string(),
                blocking: true,
                extra: Default::default(),
            }],
            ..Default::default()
        };

        let response = app
            .list_human_requests_with(
                "wf_123".to_string(),
                Some("exec_123".to_string()),
                &credential,
                &mut client,
            )
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(HUMAN_REQUEST_LIST_SCHEMA, response.schema_version);
        assert_eq!("wf_123", client.last_human_request_workflow_id.unwrap());
        assert_eq!("exec_123", client.last_human_request_execution_id.unwrap());
        assert_eq!("hr_123", response.human_requests[0].id);
    }

    #[test]
    fn human_request_resolve_sends_action_and_answer_payload() {
        let home = temp_home("human-request-resolve");
        let app = app_for_home(&home);
        let credential = credential("default", "org_123");
        let mut client = FakeManagementClient::default();

        let response = app
            .resolve_human_request_with(
                HumanRequestResolveRequest {
                    profile: None,
                    request_id: "hr_123".to_string(),
                    action: "submit".to_string(),
                    answer: serde_json::json!({
                        "approved": true,
                        "notes": "Ready"
                    }),
                },
                &credential,
                &mut client,
            )
            .unwrap();
        let payload = client.last_human_request_payload.unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(HUMAN_REQUEST_RESOLVE_SCHEMA, response.schema_version);
        assert_eq!("hr_123", client.last_human_request_id.unwrap());
        assert_eq!("submit", payload["action"]);
        assert_eq!(true, payload["answer"]["approved"]);
        assert_eq!("Ready", payload["answer"]["notes"]);
    }

    #[test]
    fn bind_workspace_success_persists_runner_project_and_workspace() {
        let home = temp_home("bind-success");
        let workspace = home.join("repo with space");
        std::fs::create_dir_all(&workspace).unwrap();
        let app = app_for_home(&home);
        let mut config = CliConfig::default();
        let credential = credential("default", "org_123");
        let mut client = FakeManagementClient {
            project: Some(Project {
                id: "prj_123".to_string(),
                organization_id: "org_123".to_string(),
                name: "Demo".to_string(),
                status: "active".to_string(),
            }),
            ..Default::default()
        };

        let response = app
            .bind_workspace_with(
                WorkspaceBindRequest {
                    profile: None,
                    project_id: "prj_123".to_string(),
                    workspace_path: workspace.to_string_lossy().to_string(),
                },
                &mut config,
                &credential,
                &mut client,
            )
            .unwrap();
        let saved = CliConfig::load_or_default(&default_config_path(&home)).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("binding_123", response.binding.id);
        assert_eq!(
            Some("binding_123".to_string()),
            saved.profiles["local"].binding_id
        );
        assert!(saved.profiles["local"]
            .workspace_path
            .as_deref()
            .unwrap()
            .contains("repo with space"));
    }

    #[test]
    fn bind_workspace_permission_denied_is_not_persisted() {
        let home = temp_home("bind-denied");
        let workspace = home.join("repo");
        std::fs::create_dir_all(&workspace).unwrap();
        let app = app_for_home(&home);
        let mut config = CliConfig::default();
        let credential = credential("default", "org_123");
        let mut client = FakeManagementClient {
            project: Some(Project {
                id: "prj_123".to_string(),
                organization_id: "org_123".to_string(),
                name: "Demo".to_string(),
                status: "active".to_string(),
            }),
            binding_error: Some(CoreError::new("MANAGEMENT_PERMISSION_DENIED", "denied")),
            ..Default::default()
        };

        let err = app
            .bind_workspace_with(
                WorkspaceBindRequest {
                    profile: None,
                    project_id: "prj_123".to_string(),
                    workspace_path: workspace.to_string_lossy().to_string(),
                },
                &mut config,
                &credential,
                &mut client,
            )
            .unwrap_err();
        let saved = CliConfig::load_or_default(&default_config_path(&home)).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("MANAGEMENT_PERMISSION_DENIED", err.code);
        assert_eq!(None, saved.profiles["default"].binding_id);
    }

    #[test]
    fn app_launch_with_existing_cli_config_shares_profile_state() {
        let home = temp_home("existing-config");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);

        let status = app.launch(AppLaunchRequest { profile: None }).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("local", status.profile);
        assert_eq!("org_123", status.organization_id.unwrap());
        assert_eq!("prj_123", status.project_id.unwrap());
        assert_eq!("binding_123", status.binding_id.unwrap());
        assert_eq!("project_bound", status.lifecycle);
    }

    #[test]
    fn runner_start_from_app_uses_selected_binding() {
        let home = temp_home("start");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);
        app.launch(AppLaunchRequest { profile: None }).unwrap();

        let status = app
            .runner_start(RunnerStartRequest { profile: None })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert!(status.runner_running);
        assert_eq!("connected", status.lifecycle);
        assert_eq!("binding_123", status.active_runner_binding_id.unwrap());
    }

    #[test]
    fn runner_stop_from_app_clears_active_core() {
        let home = temp_home("stop");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);
        app.launch(AppLaunchRequest { profile: None }).unwrap();
        app.runner_start(RunnerStartRequest { profile: None })
            .unwrap();

        let status = app
            .runner_stop(RunnerStopRequest { profile: None })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert!(!status.runner_running);
        assert_eq!("disconnected", status.lifecycle);
    }

    #[test]
    fn active_run_from_local_log_is_displayed_in_status() {
        let home = temp_home("active-run-log");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);
        std::fs::create_dir_all(app.paths.log_path.parent().unwrap()).unwrap();
        let entry =
            LogEntry::new("info", "workflow.started", "started").with_workflow_run_id("run_active");
        std::fs::write(
            &app.paths.log_path,
            format!("{}\n", serde_json::to_string(&entry).unwrap()),
        )
        .unwrap();

        let status = app.status(None).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(vec!["run_active".to_string()], status.active_runs);
        assert_eq!(1, status.active_run_count);
    }

    #[test]
    fn run_history_is_paginated_from_local_logs() {
        let home = temp_home("run-history");
        let app = app_for_home(&home);
        std::fs::create_dir_all(app.paths.log_path.parent().unwrap()).unwrap();
        let mut first =
            LogEntry::new("info", "workflow.started", "started").with_workflow_run_id("run_1");
        first.timestamp_epoch_ms = 10;
        let mut second =
            LogEntry::new("info", "workflow.completed", "done").with_workflow_run_id("run_2");
        second.timestamp_epoch_ms = 20;
        std::fs::write(
            &app.paths.log_path,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&first).unwrap(),
                serde_json::to_string(&second).unwrap()
            ),
        )
        .unwrap();

        let page = app
            .run_history(MacRunHistoryRequest {
                cursor: None,
                limit: 1,
            })
            .unwrap();
        let next = app
            .run_history(MacRunHistoryRequest {
                cursor: page.next_cursor,
                limit: 1,
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("loomex.tauri.runHistory/v1", page.schema_version);
        assert_eq!("run_2", page.items[0].run_id);
        assert_eq!("completed", page.items[0].status);
        assert_eq!("run_1", next.items[0].run_id);
    }

    #[test]
    fn reconnect_status_and_notification_status_are_exposed() {
        let home = temp_home("reconnect-notification");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);
        app.launch(AppLaunchRequest { profile: None }).unwrap();

        let reconnect = app.reconnect_status(None).unwrap();
        let notification = app.notification_status().unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("loomex.tauri.reconnectStatus/v1", reconnect.schema_version);
        assert_eq!("disconnected", reconnect.connection_indicator);
        assert_eq!(
            "loomex.tauri.notificationStatus/v1",
            notification.schema_version
        );
        assert_eq!("not_requested", notification.permission);
    }

    #[test]
    fn support_bundle_export_redacts_live_log_secrets() {
        let home = temp_home("support-bundle");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);
        std::fs::create_dir_all(app.paths.log_path.parent().unwrap()).unwrap();
        let entry = LogEntry::new("info", "runner.diagnostic", "Authorization = Bearer secret")
            .with_metadata(serde_json::json!({"token": "secret", "safe": "ok"}));
        std::fs::write(
            &app.paths.log_path,
            format!("{}\n", serde_json::to_string(&entry).unwrap()),
        )
        .unwrap();
        let output_path = home.join("bundle.json");

        let response = app
            .support_bundle(MacSupportBundleRequest {
                output_path: Some(output_path.to_string_lossy().to_string()),
                limit: 10,
            })
            .unwrap();
        let encoded = std::fs::read_to_string(&output_path).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("loomex.tauri.supportBundle/v1", response.schema_version);
        assert!(!encoded.contains("Bearer secret"));
        assert!(!encoded.contains("\"token\":\"secret\""));
        assert!(encoded.contains("[REDACTED]"));
    }

    #[test]
    fn approval_request_arrives_with_full_context() {
        let home = temp_home("approval-arrives");
        let app = app_for_home(&home);

        let approval = app
            .receive_approval_request(approval_input("approval_1", "shell.exec"))
            .unwrap();
        let list = app.approval_list().unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("approval_1", approval.approval_request_id);
        assert_eq!("run_123", approval.workflow_run_id);
        assert_eq!("node_123", approval.node_id);
        assert_eq!("shell.exec", approval.capability);
        assert_eq!("/Users/test/workspace", approval.workspace_path);
        assert_eq!(1, list.pending_count);
    }

    #[test]
    fn allow_once_sends_approval_decision() {
        let home = temp_home("approval-allow");
        let app = app_for_home(&home);
        app.receive_approval_request(approval_input("approval_1", "fs.write"))
            .unwrap();

        let outcome = app
            .approval_decide(MacApprovalDecisionRequest {
                approval_request_id: "approval_1".to_string(),
                decision: "allow_once".to_string(),
                user_id: "mac-user".to_string(),
                remember: false,
                decided_at_epoch_ms: Some(2_000),
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("approved", outcome.status);
        assert_eq!("allow_once", outcome.decision);
        assert!(!outcome.remembered);
    }

    #[test]
    fn deny_sends_approval_decision() {
        let home = temp_home("approval-deny");
        let app = app_for_home(&home);
        app.receive_approval_request(approval_input("approval_1", "http.request"))
            .unwrap();

        let outcome = app
            .approval_decide(MacApprovalDecisionRequest {
                approval_request_id: "approval_1".to_string(),
                decision: "deny".to_string(),
                user_id: "mac-user".to_string(),
                remember: false,
                decided_at_epoch_ms: Some(2_000),
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("denied", outcome.status);
        assert_eq!("deny", outcome.decision);
    }

    #[test]
    fn approval_timeout_expires_pending_dialog() {
        let home = temp_home("approval-timeout");
        let app = app_for_home(&home);
        app.receive_approval_request(approval_input("approval_1", "git.status"))
            .unwrap();

        let expired = app
            .approval_expire(MacApprovalExpireRequest {
                now_epoch_ms: 31_000,
            })
            .unwrap();
        let list = app.approval_list().unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(vec!["approval_1".to_string()], expired);
        assert_eq!("expired", list.approvals[0].status);
        assert_eq!(0, list.pending_count);
    }

    #[test]
    fn remember_disabled_by_policy_is_rejected() {
        let home = temp_home("approval-remember-disabled");
        let app = app_for_home(&home);
        app.receive_approval_request(approval_input("approval_1", "git.diff"))
            .unwrap();

        let err = app
            .approval_decide(MacApprovalDecisionRequest {
                approval_request_id: "approval_1".to_string(),
                decision: "allow_once".to_string(),
                user_id: "mac-user".to_string(),
                remember: true,
                decided_at_epoch_ms: Some(2_000),
            })
            .unwrap_err();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("TAURI_APPROVAL_REMEMBER_DISABLED", err.code);
    }

    #[test]
    fn concurrent_approval_dialogs_are_ordered_and_independent() {
        let home = temp_home("approval-concurrent");
        let app = app_for_home(&home);
        app.receive_approval_request(approval_input("approval_1", "shell.exec"))
            .unwrap();
        app.receive_approval_request(approval_input("approval_2", "fs.apply_patch"))
            .unwrap();

        let list = app.approval_list().unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(2, list.pending_count);
        assert_eq!("approval_1", list.approvals[0].approval_request_id);
        assert_eq!("approval_2", list.approvals[1].approval_request_id);
    }

    #[test]
    fn post_mvp_capability_is_displayed_as_unsupported_not_executable() {
        let home = temp_home("approval-unsupported");
        let app = app_for_home(&home);

        let approval = app
            .receive_approval_request(approval_input("approval_1", "git.push"))
            .unwrap();
        let list = app.approval_list().unwrap();
        let err = app
            .approval_decide(MacApprovalDecisionRequest {
                approval_request_id: "approval_1".to_string(),
                decision: "allow_once".to_string(),
                user_id: "mac-user".to_string(),
                remember: false,
                decided_at_epoch_ms: Some(2_000),
            })
            .unwrap_err();
        let quit = app.quit(AppQuitRequest { force: false }).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert!(approval.unsupported_post_mvp);
        assert_eq!("denied", approval.status);
        assert_eq!(0, list.pending_count);
        assert_eq!("TAURI_APPROVAL_CAPABILITY_UNSUPPORTED", err.code);
        assert_eq!(0, quit.pending_approval_count);
    }

    #[test]
    fn incomplete_approval_context_is_rejected_safe() {
        let home = temp_home("approval-incomplete");
        let app = app_for_home(&home);
        let mut input = approval_input("approval_1", "shell.exec");
        input.workspace_path.clear();

        let err = app.receive_approval_request(input).unwrap_err();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("TAURI_APPROVAL_CONTEXT_INCOMPLETE", err.code);
    }

    #[test]
    fn app_quit_with_pending_approval_requires_force() {
        let home = temp_home("approval-quit-pending");
        let app = app_for_home(&home);
        app.receive_approval_request(approval_input("approval_1", "shell.exec"))
            .unwrap();

        let err = app.quit(AppQuitRequest { force: false }).unwrap_err();
        let forced = app.quit(AppQuitRequest { force: true }).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("TAURI_QUIT_PENDING_APPROVAL", err.code);
        assert_eq!(1, forced.pending_approval_count);
    }

    #[test]
    fn live_logs_are_ordered_and_redacted() {
        let home = temp_home("live-logs");
        let app = app_for_home(&home);
        std::fs::create_dir_all(app.paths.log_path.parent().unwrap()).unwrap();
        let mut later = LogEntry::new("info", "tool_call.finished", "token=secret")
            .with_workflow_run_id("run_123")
            .with_metadata(serde_json::json!({"authorization": "Bearer secret"}));
        later.timestamp_epoch_ms = 20;
        let mut earlier = LogEntry::new("info", "tool_call.started", "started")
            .with_workflow_run_id("run_123")
            .with_metadata(serde_json::json!({"safe": "ok"}));
        earlier.timestamp_epoch_ms = 10;
        std::fs::write(
            &app.paths.log_path,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&later).unwrap(),
                serde_json::to_string(&earlier).unwrap()
            ),
        )
        .unwrap();

        let logs = app
            .live_logs(MacLiveLogsRequest {
                run_id: Some("run_123".to_string()),
                limit: 10,
            })
            .unwrap();
        let encoded = serde_json::to_string(&logs).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("tool_call.started", logs.entries[0].event_type);
        assert_eq!("tool_call.finished", logs.entries[1].event_type);
        assert!(!encoded.contains("Bearer secret"));
        assert!(!encoded.contains("token=secret"));
        assert!(encoded.contains("[REDACTED]"));
    }

    #[test]
    fn live_logs_redact_common_free_text_secret_formats() {
        let home = temp_home("live-logs-secret-formats");
        let app = app_for_home(&home);
        std::fs::create_dir_all(app.paths.log_path.parent().unwrap()).unwrap();
        let secret_messages = [
            r#"{"token": "json-secret"}"#,
            "password: pass-secret",
            "Authorization = Bearer auth-secret",
            "api-key: key-secret",
            "cookie: session=cookie-secret",
        ];
        let entries = secret_messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                let mut entry = LogEntry::new("warn", "tool_call.output", *message)
                    .with_workflow_run_id("run_123");
                entry.timestamp_epoch_ms = index as u64;
                serde_json::to_string(&entry).unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&app.paths.log_path, format!("{entries}\n")).unwrap();

        let logs = app
            .live_logs(MacLiveLogsRequest {
                run_id: Some("run_123".to_string()),
                limit: 10,
            })
            .unwrap();
        let encoded = serde_json::to_string(&logs).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        for leaked in [
            "json-secret",
            "pass-secret",
            "auth-secret",
            "key-secret",
            "cookie-secret",
        ] {
            assert!(!encoded.contains(leaked), "{leaked} leaked in {encoded}");
        }
        assert_eq!(secret_messages.len(), encoded.matches("[REDACTED]").count());
    }

    #[test]
    fn approval_details_are_redacted_for_common_secret_formats() {
        let home = temp_home("approval-secret-redaction");
        let app = app_for_home(&home);
        let mut input = approval_input("approval_1", "shell.exec");
        input.action_summary = "Run with api-key: summary-secret".to_string();
        input.full_request_details =
            r#"POST /deploy {"token": "detail-secret"} Authorization = Bearer auth-secret"#
                .to_string();

        let approval = app.receive_approval_request(input).unwrap();
        let encoded = serde_json::to_string(&approval).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("[REDACTED]", approval.action_summary);
        assert_eq!("[REDACTED]", approval.full_request_details);
        assert!(!encoded.contains("summary-secret"));
        assert!(!encoded.contains("detail-secret"));
        assert!(!encoded.contains("auth-secret"));
    }

    #[test]
    fn logout_clears_credentials_binding_and_app_state() {
        let home = temp_home("logout");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);
        let mut config = CliConfig::load_or_default(&default_config_path(&home)).unwrap();
        let store = LocalCredentialStore::new(home.join(".loomex").join("credentials"));
        store
            .save(&credential("local", "org_123"))
            .expect("credential should save");
        app.runner_start(RunnerStartRequest { profile: None })
            .unwrap();

        let status = app
            .logout_with_store(None, &mut config, &store)
            .expect("logout should clear state");
        let saved_config = CliConfig::load_or_default(&default_config_path(&home)).unwrap();
        let loaded = store.load("local").unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert!(!status.authenticated);
        assert!(!status.runner_running);
        assert_eq!(None, loaded);
        assert_eq!(None, saved_config.profiles["local"].binding_id);
        assert_eq!(None, saved_config.profiles["local"].workspace_path);
    }

    #[test]
    fn duplicate_runner_core_for_same_binding_is_idempotent() {
        let home = temp_home("duplicate");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);
        app.launch(AppLaunchRequest { profile: None }).unwrap();
        app.runner_start(RunnerStartRequest { profile: None })
            .unwrap();

        let status = app
            .runner_start(RunnerStartRequest { profile: None })
            .unwrap();
        app.runner_stop(RunnerStopRequest { profile: None })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert!(status.runner_running);
        assert!(status.runner_service_persistent);
        assert_eq!(
            Some("tauri_started".to_string()),
            status.runner_service_origin
        );
        assert_eq!("binding_123", status.active_runner_binding_id.unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn launch_attaches_to_compatible_persistent_runner_service() {
        let home = temp_home("attach-existing-service");
        write_bound_cli_config(&home);
        let workspace = home.join("workspace");
        let service = MockLocalRunnerService::start(&home, "binding_123", &workspace);
        let app = app_for_home(&home);

        let status = app.launch(AppLaunchRequest { profile: None }).unwrap();
        let started = app
            .runner_start(RunnerStartRequest { profile: None })
            .unwrap();

        assert!(status.runner_running);
        assert!(status.runner_service_persistent);
        assert_eq!(Some("attached".to_string()), status.runner_service_origin);
        assert_eq!(Some("attached".to_string()), started.runner_service_origin);

        drop(service);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn duplicate_runner_core_across_app_instances_is_guarded_by_binding_lock() {
        let home = temp_home("duplicate-cross-instance");
        write_bound_cli_config(&home);
        let first = app_for_home(&home);
        let second = app_for_home(&home);
        first.launch(AppLaunchRequest { profile: None }).unwrap();
        second.launch(AppLaunchRequest { profile: None }).unwrap();
        first
            .runner_start(RunnerStartRequest { profile: None })
            .unwrap();

        let err = second
            .runner_start(RunnerStartRequest { profile: None })
            .unwrap_err();
        first
            .runner_stop(RunnerStopRequest { profile: None })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("RUNNER_RUNTIME_GUARD_CONFLICT", err.code);
    }

    #[test]
    fn cli_style_existing_binding_guard_blocks_app_start() {
        let home = temp_home("duplicate-cli");
        write_bound_cli_config(&home);
        let config_path = default_config_path(&home);
        let guard_path = runner_runtime_guard_path(&config_path, "binding_123");
        std::fs::create_dir_all(guard_path.parent().unwrap()).unwrap();
        std::fs::write(
            &guard_path,
            format!(
                "surface=loomex-cli\npid={}\nbinding_id=binding_123\n",
                std::process::id()
            ),
        )
        .unwrap();
        let app = app_for_home(&home);
        app.launch(AppLaunchRequest { profile: None }).unwrap();

        let err = app
            .runner_start(RunnerStartRequest { profile: None })
            .unwrap_err();
        let _ = std::fs::remove_file(&guard_path);
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!("RUNNER_RUNTIME_GUARD_CONFLICT", err.code);
    }

    #[test]
    fn stale_dead_pid_guard_is_cleaned_before_app_start() {
        let home = temp_home("stale-dead-pid");
        write_bound_cli_config(&home);
        let config_path = default_config_path(&home);
        let guard_path = runner_runtime_guard_path(&config_path, "binding_123");
        std::fs::create_dir_all(guard_path.parent().unwrap()).unwrap();
        std::fs::write(
            &guard_path,
            "surface=old-cli\npid=4294967295\nbinding_id=binding_123\n",
        )
        .unwrap();
        let app = app_for_home(&home);
        app.launch(AppLaunchRequest { profile: None }).unwrap();

        let status = app
            .runner_start(RunnerStartRequest { profile: None })
            .unwrap();
        app.runner_stop(RunnerStopRequest { profile: None })
            .unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert!(status.runner_running);
        assert_eq!("binding_123", status.active_runner_binding_id.unwrap());
    }

    #[test]
    fn tauri_capability_exposes_registered_commands() {
        let capability = std::fs::read_to_string("capabilities/default.json").unwrap();
        let permissions = std::fs::read_to_string("permissions/commands.toml").unwrap();

        for command in [
            "app_launch",
            "login_device_start",
            "login_device_complete",
            "login_cancel",
            "login_api_key",
            "login_workspace",
            "organization_list",
            "project_list",
            "project_select",
            "workspace_pick_directory",
            "workspace_file_list",
            "workspace_file_read",
            "terminal_command",
            "workspace_set",
            "workspace_bind",
            "workspace_picker_cancel",
            "workflow_list",
            "workflow_run_chat",
            "workflow_run_detail",
            "runner_status",
            "runner_start",
            "runner_stop",
            "approval_list",
            "approval_decide",
            "approval_expire",
            "run_approval_cancel",
            "live_logs",
            "run_history",
            "reconnect_status",
            "notification_status",
            "support_bundle_export",
            "open_in_loomex_url",
            "logout",
            "app_quit",
        ] {
            assert!(permissions.contains(command));
        }
        for permission in [
            "allow-app-launch",
            "allow-login-device-start",
            "allow-login-device-complete",
            "allow-login-cancel",
            "allow-login-api-key",
            "allow-login-workspace",
            "allow-organization-list",
            "allow-project-list",
            "allow-project-select",
            "allow-workspace-pick-directory",
            "allow-workspace-file-list",
            "allow-workspace-file-read",
            "allow-terminal-command",
            "allow-workspace-set",
            "allow-workspace-bind",
            "allow-workspace-picker-cancel",
            "allow-workflow-list",
            "allow-workflow-run-chat",
            "allow-workflow-run-detail",
            "allow-runner-status",
            "allow-runner-start",
            "allow-runner-stop",
            "allow-approval-list",
            "allow-approval-decide",
            "allow-approval-expire",
            "allow-run-approval-cancel",
            "allow-live-logs",
            "allow-run-history",
            "allow-reconnect-status",
            "allow-notification-status",
            "allow-support-bundle-export",
            "allow-open-in-loomex-url",
            "allow-logout",
            "allow-app-quit",
        ] {
            assert!(capability.contains(permission));
        }
        assert!(capability.contains("dialog:allow-open"));
    }

    #[test]
    fn ui_uses_native_workspace_picker_command_not_web_file_input() {
        let ui = std::fs::read_to_string("ui/index.html").unwrap();

        assert!(ui.contains("workspace_pick_directory"));
        assert!(ui.contains("workspace_set"));
        assert!(!ui.contains("webkitdirectory"));
        assert!(!ui.contains("type=\"file\""));
    }

    #[test]
    fn static_ui_global_invoke_has_matching_tauri_config() {
        let ui = std::fs::read_to_string("ui/index.html").unwrap();
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string("tauri.conf.json").unwrap()).unwrap();

        assert!(ui.contains("window.__TAURI__?.core?.invoke"));
        for command in [
            "login_workspace",
            "workspace_file_list",
            "workflow_list",
            "workflow_run_chat",
            "workflow_run_detail",
            "workspace_set",
            "approval_list",
        ] {
            assert!(ui.contains(command));
        }
        assert!(!ui.contains("Organization ID"));
        assert_eq!(Some(true), config["app"]["withGlobalTauri"].as_bool());
    }

    #[test]
    fn mac_packaging_config_declares_installable_app_and_dmg() {
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string("tauri.conf.json").unwrap()).unwrap();
        let targets = config["bundle"]["targets"].as_array().unwrap();

        assert_eq!("app.loomex.runner", config["identifier"]);
        assert_eq!(Some(true), config["bundle"]["active"].as_bool());
        assert_eq!("DeveloperTool", config["bundle"]["category"]);
        assert!(targets.iter().any(|target| target == "app"));
        assert!(targets.iter().any(|target| target == "dmg"));
    }

    #[test]
    fn mac_packaging_smoke_script_covers_build_sign_checksum_install_and_launch() {
        let script = std::fs::read_to_string("../../scripts/mac_packaging_smoke.sh").unwrap();

        for expected in [
            "cargo build -p loomex-tauri",
            "cargo build -p loomex-cli",
            "Contents/MacOS/loomex",
            "Loomex.app",
            "hdiutil create",
            "codesign --force --deep --sign",
            "codesign --verify --deep --strict",
            "shasum -a 256",
            "SHA256SUMS",
            "LOOMEX_TAURI_SMOKE_LAUNCH",
            "open -n",
            "packaging-smoke.json",
            "system_keychain_when_available",
            "uses_tauri_native_dialog_command",
            "uses_system_browser_url_from_login_device_start",
        ] {
            assert!(script.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn mac_packaging_doc_records_notarization_update_and_restart_risks() {
        let doc = std::fs::read_to_string("../../docs/mac-packaging-signing-smoke.md").unwrap();

        for expected in [
            "Developer ID Application",
            "xcrun notarytool submit --wait",
            "xcrun stapler staple",
            "Gatekeeper",
            "Downloads",
            "signed update manifest",
            "no deletion of `~/.loomex/config.toml`",
            "Quit and relaunch the app",
            "workspace_pick_directory",
            "SystemCredentialStore",
        ] {
            assert!(doc.contains(expected), "missing {expected}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn app_quit_with_active_run_detaches_without_stopping_persistent_service() {
        let home = temp_home("quit-active-run");
        write_bound_cli_config(&home);
        let workspace = home.join("workspace");
        let service = MockLocalRunnerService::start(&home, "binding_123", &workspace);
        let app = app_for_home(&home);
        app.launch(AppLaunchRequest { profile: None }).unwrap();
        app.set_active_run_count_for_test(1).unwrap();

        let quit = app.quit(AppQuitRequest { force: false }).unwrap();
        let service_status = probe_local_runner_service(&app.paths).unwrap().unwrap();
        let reattached = app.launch(AppLaunchRequest { profile: None }).unwrap();

        assert!(!quit.runner_running);
        assert!(service_status.running);
        assert_eq!(
            Some("attached".to_string()),
            reattached.runner_service_origin
        );

        drop(service);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(unix)]
    #[test]
    fn app_quit_does_not_kill_service_started_by_tauri() {
        let home = temp_home("quit-tauri-started-service");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);
        app.launch(AppLaunchRequest { profile: None }).unwrap();
        app.runner_start(RunnerStartRequest { profile: None })
            .unwrap();
        app.set_active_run_count_for_test(1).unwrap();
        let guard_path = runner_runtime_guard_path(&app.paths.config_path, "binding_123");
        let pid = loomex_core::read_runner_runtime_guard(&guard_path)
            .unwrap()
            .unwrap()
            .pid;

        let quit = app.quit(AppQuitRequest { force: false }).unwrap();
        let guard_error =
            acquire_runner_runtime_guard(&app.paths.config_path, "binding_123", "test-probe")
                .unwrap_err();

        assert!(!quit.runner_running);
        assert_eq!("RUNNER_RUNTIME_GUARD_CONFLICT", guard_error.code);

        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .status();
        for _ in 0..50 {
            if !guard_path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn background_and_tray_lifecycle_keeps_shared_state() {
        let home = temp_home("background");
        write_bound_cli_config(&home);
        let app = app_for_home(&home);
        app.launch(AppLaunchRequest { profile: None }).unwrap();

        let background = app.set_backgrounded(true).unwrap();
        let foreground = app.set_backgrounded(false).unwrap();
        let _ = std::fs::remove_dir_all(&home);

        assert!(background.backgrounded);
        assert!(!foreground.backgrounded);
        assert_eq!("binding_123", foreground.binding_id.unwrap());
    }
}
