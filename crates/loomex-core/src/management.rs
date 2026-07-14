use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CoreError, CoreResult};

const RUNNER_AUTH_EXCHANGE_PATH: &str = "/runner-control/runner/v1/auth/exchange/";
const RUNNER_AUTH_BOOTSTRAP_PATH: &str = "/runner-control/runner/v1/auth/bootstrap/";
const RUNNER_TOKEN_NON_EXPIRING_EXPIRES_AT: &str = "9999-12-31T23:59:59Z";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLoginStartRequest {
    pub client_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLoginChallenge {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in_seconds: u64,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthTokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyExchangeResult {
    pub token: AuthTokenResponse,
    pub organization_id: Option<String>,
    pub project_id: Option<String>,
    pub runner_id: Option<String>,
    pub binding_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceLoginResult {
    pub token: String,
    pub organization_id: Option<String>,
    pub project_id: Option<String>,
}

impl ApiKeyExchangeResult {
    pub fn from_token(token: AuthTokenResponse) -> Self {
        Self {
            token,
            organization_id: None,
            project_id: None,
            runner_id: None,
            binding_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runner {
    pub id: String,
    pub organization_id: String,
    pub status: String,
    pub runner_version: String,
    pub protocol_version: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerUpsertRequest {
    pub organization_id: String,
    pub display_name: String,
    pub machine_fingerprint_hash: String,
    pub os: String,
    pub arch: String,
    pub runner_version: String,
    pub protocol_version: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRunnerBindingCreateRequest {
    pub organization_id: String,
    pub runner_id: String,
    pub local_root_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_root_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagementProjectRunnerBinding {
    pub id: String,
    pub organization_id: String,
    pub project_id: String,
    pub runner_id: String,
    pub local_root_path: String,
    pub status: String,
    #[serde(default)]
    pub local_root_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunStartRequest {
    pub organization_id: String,
    pub project_id: String,
    pub workflow_id: String,
    pub inputs: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_runner_binding_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRunStartResponse {
    pub id: String,
    pub status: String,
    #[serde(default, rename = "uiUrl", alias = "ui_url")]
    pub ui_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamCredentialRequest {
    pub organization_id: String,
    pub project_id: String,
    pub runner_id: String,
    pub project_runner_binding_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_session_id: Option<String>,
    pub protocol_version: String,
    pub runner_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamCredentialResponse {
    pub stream_token: String,
    pub token_type: String,
    pub audience: String,
    pub runner_session_id: String,
    pub expires_at: String,
    pub grpc_endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanRequestSummary {
    pub id: String,
    pub status: String,
    pub title: String,
    #[serde(default)]
    pub execution: Option<HumanRequestExecution>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerWorkflowSummary {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default, rename = "activeVersionId", alias = "active_version_id")]
    pub active_version_id: Option<String>,
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerWorkflowExecutionResponse {
    pub execution: Value,
    #[serde(default, rename = "humanRequest", alias = "human_request")]
    pub human_request: Option<Value>,
    #[serde(default)]
    pub runner: Option<Value>,
    #[serde(default)]
    pub events: Vec<Value>,
    #[serde(default, rename = "aiTrace", alias = "ai_trace")]
    pub ai_trace: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerWorkflowExecutionListResponse {
    #[serde(default)]
    pub executions: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerWorkflowInputSchemaResponse {
    #[serde(default, rename = "inputSchema", alias = "input_schema")]
    pub input_schema: Option<Value>,
    #[serde(default, rename = "activeVersion", alias = "active_version")]
    pub active_version: Option<Value>,
    #[serde(default, rename = "selectedVersion", alias = "selected_version")]
    pub selected_version: Option<Value>,
    #[serde(default)]
    pub versions: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerSessionResponse {
    pub runner: Value,
    pub session: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerJobResponse {
    pub job: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerJobEventCreateResponse {
    #[serde(default)]
    pub events: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanRequestExecution {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HumanRequestResolveResponse {
    #[serde(rename = "requestId")]
    pub request_id: String,
    #[serde(rename = "requestStatus")]
    pub request_status: String,
    #[serde(default, rename = "executionId")]
    pub execution_id: Option<String>,
    #[serde(default, rename = "executionStatus")]
    pub execution_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct OrganizationList {
    items: Vec<Organization>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProjectList {
    items: Vec<Project>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProjectRunnerBindingList {
    items: Vec<ManagementProjectRunnerBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ClientWorkflowRunStartRequest {
    input: Value,
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "projectRunnerBindingId"
    )]
    project_runner_binding_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RunnerWorkflowListResponse {
    workflows: Vec<RunnerWorkflowSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerHumanRequestListResponse {
    #[serde(default)]
    human_requests: Vec<HumanRequestSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RunnerWorkflowExecutionStartRequest {
    inputs: Value,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sessionId")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClientEnvelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerAuthExchangeData {
    runner: RunnerAuthRunner,
    runner_token: String,
    token_type: String,
    organization_id: String,
    #[serde(default)]
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceLoginData {
    token: String,
    #[serde(default)]
    organization: Option<WorkspaceOrganizationData>,
    #[serde(default)]
    projects: Vec<WorkspaceProjectData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceOrganizationData {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceProjectData {
    id: String,
    #[serde(default)]
    organization_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunnerAuthRunner {
    id: String,
    #[serde(default)]
    project_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClientWorkflowDetailResponse {
    #[serde(default, rename = "activeVersion")]
    active_version: Option<ClientWorkflowVersion>,
}

#[derive(Debug, Deserialize)]
struct ClientWorkflowVersion {
    #[serde(default)]
    definition: Value,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ManagementCredential {
    pub profile: String,
    pub organization_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_at: String,
    pub storage_backend: CredentialStorageBackend,
    pub storage_warning: Option<String>,
}

impl std::fmt::Debug for ManagementCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagementCredential")
            .field("profile", &self.profile)
            .field("organization_id", &self.organization_id)
            .field("access_token", &"[REDACTED]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("token_type", &self.token_type)
            .field("expires_at", &self.expires_at)
            .field("storage_backend", &self.storage_backend)
            .field("storage_warning", &self.storage_warning)
            .finish()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialStorageBackend {
    MacOsKeychain,
    LocalFileFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialStorageOutcome {
    pub backend: CredentialStorageBackend,
    pub warning: Option<String>,
}

impl CredentialStorageOutcome {
    pub fn for_backend(backend: CredentialStorageBackend) -> Self {
        Self {
            backend,
            warning: storage_warning_for_backend(backend),
        }
    }
}

impl ManagementCredential {
    pub fn from_token_response(
        profile: impl Into<String>,
        organization_id: impl Into<String>,
        token: AuthTokenResponse,
        storage_backend: CredentialStorageBackend,
    ) -> CoreResult<Self> {
        validate_auth_token(&token)?;
        let storage_warning = storage_warning_for_backend(storage_backend);
        Ok(Self {
            profile: profile.into(),
            organization_id: organization_id.into(),
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            token_type: token.token_type,
            expires_at: token.expires_at,
            storage_backend,
            storage_warning,
        })
    }

    pub fn validate_not_expiring(
        &self,
        now_epoch_seconds: u64,
        clock_skew_seconds: u64,
    ) -> CoreResult<()> {
        let expires_at = parse_rfc3339_utc_epoch_seconds(&self.expires_at)?;
        if expires_at <= now_epoch_seconds.saturating_add(clock_skew_seconds) {
            return Err(CoreError::new(
                "AUTH_TOKEN_EXPIRED",
                "management token is expired or too close to expiry; refresh endpoint is not available in the current management API contract",
            ));
        }
        Ok(())
    }
}

fn storage_warning_for_backend(backend: CredentialStorageBackend) -> Option<String> {
    match backend {
        CredentialStorageBackend::MacOsKeychain => None,
        CredentialStorageBackend::LocalFileFallback => Some(
            "secure OS credential storage unavailable; token stored in restricted local fallback"
                .to_string(),
        ),
    }
}

fn validate_auth_token(token: &AuthTokenResponse) -> CoreResult<()> {
    if token.access_token.trim().is_empty() {
        return Err(CoreError::new(
            "AUTH_TOKEN_INVALID",
            "access_token is required",
        ));
    }
    if token.token_type != "Bearer" {
        return Err(CoreError::new(
            "AUTH_TOKEN_INVALID",
            "token_type must be Bearer",
        ));
    }
    if token.expires_at.trim().is_empty() {
        return Err(CoreError::new(
            "AUTH_TOKEN_INVALID",
            "expires_at is required",
        ));
    }
    Ok(())
}

pub trait CredentialStore {
    fn save(&self, credential: &ManagementCredential) -> CoreResult<CredentialStorageOutcome>;
    fn load(&self, profile: &str) -> CoreResult<Option<ManagementCredential>>;
    fn delete(&self, profile: &str) -> CoreResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCredentialStore {
    root_dir: PathBuf,
}

impl LocalCredentialStore {
    pub fn new(root_dir: PathBuf) -> Self {
        Self { root_dir }
    }

    fn path_for_profile(&self, profile: &str) -> CoreResult<PathBuf> {
        if profile.trim().is_empty() || profile.contains('/') || profile.contains('\\') {
            return Err(CoreError::new("CREDENTIAL_PROFILE_INVALID", profile));
        }
        Ok(self.root_dir.join(format!("{profile}.json")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemCredentialStore {
    keychain: Option<MacOsKeychainCredentialStore>,
    fallback: LocalCredentialStore,
}

impl SystemCredentialStore {
    pub fn new(fallback_root_dir: PathBuf) -> Self {
        Self {
            keychain: MacOsKeychainCredentialStore::available(),
            fallback: LocalCredentialStore::new(fallback_root_dir),
        }
    }

    pub fn storage_backend(&self) -> CredentialStorageBackend {
        if self.keychain.is_some() {
            CredentialStorageBackend::MacOsKeychain
        } else {
            CredentialStorageBackend::LocalFileFallback
        }
    }
}

impl CredentialStore for SystemCredentialStore {
    fn save(&self, credential: &ManagementCredential) -> CoreResult<CredentialStorageOutcome> {
        if let Some(keychain) = &self.keychain {
            let mut keychain_credential = credential.clone();
            keychain_credential.storage_backend = CredentialStorageBackend::MacOsKeychain;
            keychain_credential.storage_warning = None;
            if let Ok(outcome) = keychain.save(&keychain_credential) {
                let _ = self.fallback.delete(&credential.profile);
                return Ok(outcome);
            }
        }
        let mut fallback_credential = credential.clone();
        fallback_credential.storage_backend = CredentialStorageBackend::LocalFileFallback;
        fallback_credential.storage_warning =
            storage_warning_for_backend(CredentialStorageBackend::LocalFileFallback);
        self.fallback.save(&fallback_credential)
    }

    fn load(&self, profile: &str) -> CoreResult<Option<ManagementCredential>> {
        if let Some(keychain) = &self.keychain {
            match keychain.load(profile) {
                Ok(Some(credential)) => return Ok(Some(credential)),
                Ok(None) => {}
                Err(_) => {}
            }
        }
        self.fallback.load(profile)
    }

    fn delete(&self, profile: &str) -> CoreResult<()> {
        if let Some(keychain) = &self.keychain {
            let _ = keychain.delete(profile);
        }
        self.fallback.delete(profile)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MacOsKeychainCredentialStore {
    service: String,
}

impl MacOsKeychainCredentialStore {
    fn available() -> Option<Self> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let Ok(output) = Command::new("security").arg("help").output() else {
            return None;
        };
        if !output.status.success() {
            return None;
        }
        Some(Self {
            service: "app.loomex.cli.management".to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalCredentialDocument {
    schema_version: String,
    profile: String,
    organization_id: String,
    access_token_b64: String,
    refresh_token_b64: Option<String>,
    token_type: String,
    expires_at: String,
    storage_backend: CredentialStorageBackend,
}

fn credential_to_document(credential: &ManagementCredential) -> LocalCredentialDocument {
    LocalCredentialDocument {
        schema_version: "loomex.cli.credential/v1".to_string(),
        profile: credential.profile.clone(),
        organization_id: credential.organization_id.clone(),
        access_token_b64: BASE64.encode(credential.access_token.as_bytes()),
        refresh_token_b64: credential
            .refresh_token
            .as_ref()
            .map(|value| BASE64.encode(value.as_bytes())),
        token_type: credential.token_type.clone(),
        expires_at: credential.expires_at.clone(),
        storage_backend: credential.storage_backend,
    }
}

fn credential_from_document(document: LocalCredentialDocument) -> CoreResult<ManagementCredential> {
    let access_token = decode_secret(&document.access_token_b64)?;
    let refresh_token = document
        .refresh_token_b64
        .as_deref()
        .map(decode_secret)
        .transpose()?;
    Ok(ManagementCredential {
        profile: document.profile,
        organization_id: document.organization_id,
        access_token,
        refresh_token,
        token_type: document.token_type,
        expires_at: document.expires_at,
        storage_backend: document.storage_backend,
        storage_warning: storage_warning_for_backend(document.storage_backend),
    })
}

impl CredentialStore for LocalCredentialStore {
    fn save(&self, credential: &ManagementCredential) -> CoreResult<CredentialStorageOutcome> {
        fs::create_dir_all(&self.root_dir)
            .map_err(|err| CoreError::new("CREDENTIAL_STORE_WRITE_FAILED", err.to_string()))?;
        set_private_dir_permissions(&self.root_dir)?;
        let document = credential_to_document(credential);
        let payload = serde_json::to_vec_pretty(&document)
            .map_err(|err| CoreError::new("CREDENTIAL_STORE_WRITE_FAILED", err.to_string()))?;
        let path = self.path_for_profile(&credential.profile)?;
        let temp_path = path.with_extension("json.tmp");
        fs::write(&temp_path, payload)
            .map_err(|err| CoreError::new("CREDENTIAL_STORE_WRITE_FAILED", err.to_string()))?;
        set_private_file_permissions(&temp_path)?;
        fs::rename(&temp_path, &path)
            .map_err(|err| CoreError::new("CREDENTIAL_STORE_WRITE_FAILED", err.to_string()))?;
        Ok(CredentialStorageOutcome::for_backend(
            credential.storage_backend,
        ))
    }

    fn load(&self, profile: &str) -> CoreResult<Option<ManagementCredential>> {
        let path = self.path_for_profile(profile)?;
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read(&path)
            .map_err(|err| CoreError::new("CREDENTIAL_STORE_READ_FAILED", err.to_string()))?;
        let document: LocalCredentialDocument = serde_json::from_slice(&content)
            .map_err(|err| CoreError::new("CREDENTIAL_STORE_PARSE_FAILED", err.to_string()))?;
        credential_from_document(document).map(Some)
    }

    fn delete(&self, profile: &str) -> CoreResult<()> {
        let path = self.path_for_profile(profile)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(CoreError::new(
                "CREDENTIAL_STORE_DELETE_FAILED",
                err.to_string(),
            )),
        }
    }
}

impl CredentialStore for MacOsKeychainCredentialStore {
    fn save(&self, credential: &ManagementCredential) -> CoreResult<CredentialStorageOutcome> {
        let document = credential_to_document(credential);
        let payload = serde_json::to_string(&document)
            .map_err(|err| CoreError::new("CREDENTIAL_STORE_WRITE_FAILED", err.to_string()))?;
        let output = Command::new("security")
            .args([
                "add-generic-password",
                "-U",
                "-s",
                &self.service,
                "-a",
                &credential.profile,
                "-w",
                &payload,
            ])
            .output()
            .map_err(|err| CoreError::new("CREDENTIAL_STORE_WRITE_FAILED", err.to_string()))?;
        if output.status.success() {
            Ok(CredentialStorageOutcome::for_backend(
                CredentialStorageBackend::MacOsKeychain,
            ))
        } else {
            Err(CoreError::new(
                "CREDENTIAL_STORE_WRITE_FAILED",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ))
        }
    }

    fn load(&self, profile: &str) -> CoreResult<Option<ManagementCredential>> {
        let output = Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                &self.service,
                "-a",
                profile,
                "-w",
            ])
            .output()
            .map_err(|err| CoreError::new("CREDENTIAL_STORE_READ_FAILED", err.to_string()))?;
        if !output.status.success() {
            return Ok(None);
        }
        let document: LocalCredentialDocument = serde_json::from_slice(&output.stdout)
            .map_err(|err| CoreError::new("CREDENTIAL_STORE_PARSE_FAILED", err.to_string()))?;
        credential_from_document(document).map(Some)
    }

    fn delete(&self, profile: &str) -> CoreResult<()> {
        let _ = Command::new("security")
            .args([
                "delete-generic-password",
                "-s",
                &self.service,
                "-a",
                profile,
            ])
            .output();
        Ok(())
    }
}

fn decode_secret(value: &str) -> CoreResult<String> {
    let bytes = BASE64
        .decode(value)
        .map_err(|err| CoreError::new("CREDENTIAL_STORE_PARSE_FAILED", err.to_string()))?;
    String::from_utf8(bytes)
        .map_err(|err| CoreError::new("CREDENTIAL_STORE_PARSE_FAILED", err.to_string()))
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> CoreResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|err| CoreError::new("CREDENTIAL_STORE_WRITE_FAILED", err.to_string()))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> CoreResult<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> CoreResult<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|err| CoreError::new("CREDENTIAL_STORE_WRITE_FAILED", err.to_string()))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> CoreResult<()> {
    Ok(())
}

pub trait ManagementApiClient {
    fn start_device_login(&mut self) -> CoreResult<DeviceLoginChallenge>;
    fn poll_device_token(&mut self, device_code: &str) -> CoreResult<Option<AuthTokenResponse>>;
    fn exchange_api_key(
        &mut self,
        api_key: &str,
        api_secret: &str,
        organization_id: &str,
    ) -> CoreResult<ApiKeyExchangeResult>;
    fn login_workspace(&mut self, email: &str, password: &str) -> CoreResult<WorkspaceLoginResult>;
    fn bootstrap_runner_with_workspace_token(
        &mut self,
        workspace_token: &str,
        organization_id: &str,
        project_id: Option<&str>,
        workspace_root: Option<&str>,
    ) -> CoreResult<ApiKeyExchangeResult>;
    fn list_organizations(
        &mut self,
        credential: &ManagementCredential,
    ) -> CoreResult<Vec<Organization>>;
    fn list_projects(
        &mut self,
        credential: &ManagementCredential,
        organization_id: &str,
    ) -> CoreResult<Vec<Project>>;
    fn get_project(
        &mut self,
        credential: &ManagementCredential,
        project_id: &str,
    ) -> CoreResult<Project>;
    fn get_current_runner(
        &mut self,
        credential: &ManagementCredential,
        organization_id: &str,
    ) -> CoreResult<Runner>;
    fn upsert_current_runner(
        &mut self,
        credential: &ManagementCredential,
        request: &RunnerUpsertRequest,
        idempotency_key: &str,
    ) -> CoreResult<Runner>;
    fn create_project_runner_binding(
        &mut self,
        credential: &ManagementCredential,
        project_id: &str,
        request: &ProjectRunnerBindingCreateRequest,
        idempotency_key: &str,
    ) -> CoreResult<ManagementProjectRunnerBinding>;
    fn list_project_runner_bindings(
        &mut self,
        credential: &ManagementCredential,
        project_id: &str,
    ) -> CoreResult<Vec<ManagementProjectRunnerBinding>>;
    fn revoke_project_runner_binding(
        &mut self,
        credential: &ManagementCredential,
        project_id: &str,
        binding_id: &str,
        idempotency_key: &str,
    ) -> CoreResult<()>;
    fn start_workflow_run(
        &mut self,
        credential: &ManagementCredential,
        request: &WorkflowRunStartRequest,
    ) -> CoreResult<WorkflowRunStartResponse>;
    fn list_runner_workflows(
        &mut self,
        credential: &ManagementCredential,
    ) -> CoreResult<Vec<RunnerWorkflowSummary>>;
    fn start_runner_workflow_execution(
        &mut self,
        credential: &ManagementCredential,
        workflow_id: &str,
        inputs: Value,
        session_id: Option<&str>,
        version: Option<&str>,
    ) -> CoreResult<RunnerWorkflowExecutionResponse>;
    fn list_runner_workflow_executions(
        &mut self,
        credential: &ManagementCredential,
        workflow_id: &str,
        limit: usize,
    ) -> CoreResult<RunnerWorkflowExecutionListResponse>;
    fn get_runner_workflow_input_schema(
        &mut self,
        credential: &ManagementCredential,
        workflow_id: &str,
        version: Option<&str>,
    ) -> CoreResult<RunnerWorkflowInputSchemaResponse>;
    fn get_runner_workflow_execution(
        &mut self,
        credential: &ManagementCredential,
        execution_id: &str,
    ) -> CoreResult<RunnerWorkflowExecutionResponse>;
    fn get_workflow_input_schema(
        &mut self,
        credential: &ManagementCredential,
        workflow_id: &str,
    ) -> CoreResult<Option<Value>>;
    fn list_human_requests(
        &mut self,
        credential: &ManagementCredential,
        workflow_id: &str,
        execution_id: Option<&str>,
    ) -> CoreResult<Vec<HumanRequestSummary>>;
    fn resolve_human_request(
        &mut self,
        credential: &ManagementCredential,
        request_id: &str,
        payload: &Value,
    ) -> CoreResult<HumanRequestResolveResponse>;
    fn create_runner_session(
        &mut self,
        credential: &ManagementCredential,
        workspace_root: &str,
        manifest: Value,
        transport: &str,
    ) -> CoreResult<RunnerSessionResponse>;
    fn heartbeat_runner_session(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
        manifest: Value,
    ) -> CoreResult<RunnerSessionResponse>;
    fn lease_runner_job(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
    ) -> CoreResult<RunnerJobResponse>;
    fn start_runner_job(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
        job_id: &str,
    ) -> CoreResult<RunnerJobResponse>;
    fn append_runner_job_events(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
        job_id: &str,
        events: Vec<Value>,
    ) -> CoreResult<RunnerJobEventCreateResponse>;
    fn complete_runner_job(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
        job_id: &str,
        result: Value,
    ) -> CoreResult<RunnerJobResponse>;
    fn fail_runner_job(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
        job_id: &str,
        error: Value,
    ) -> CoreResult<RunnerJobResponse>;
    fn issue_stream_credential(
        &mut self,
        credential: &ManagementCredential,
        request: &StreamCredentialRequest,
        idempotency_key: &str,
    ) -> CoreResult<StreamCredentialResponse>;
}

#[derive(Debug, Clone)]
pub struct HttpManagementApiClient {
    base_url: String,
    host_header: Option<String>,
    client: Client,
}

impl HttpManagementApiClient {
    pub fn new(server_url: impl Into<String>, host_header: Option<String>) -> CoreResult<Self> {
        let mut base_url = server_url.into().trim_end_matches('/').to_string();
        if !base_url.ends_with("/api/v1") {
            base_url.push_str("/api/v1");
        }
        Ok(Self {
            base_url,
            host_header,
            client: Client::builder()
                .build()
                .map_err(|err| CoreError::new("MANAGEMENT_HTTP_CLIENT_FAILED", err.to_string()))?,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn apply_common_headers(
        &self,
        mut request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        if let Some(host_header) = &self.host_header {
            request = request.header("Host", host_header);
        }
        request
    }

    fn get_with_auth(
        &self,
        path: &str,
        credential: &ManagementCredential,
    ) -> reqwest::blocking::RequestBuilder {
        self.apply_common_headers(
            self.client
                .get(self.url(path))
                .bearer_auth(&credential.access_token),
        )
    }

    fn post_with_auth(
        &self,
        path: &str,
        credential: &ManagementCredential,
    ) -> reqwest::blocking::RequestBuilder {
        self.apply_common_headers(
            self.client
                .post(self.url(path))
                .bearer_auth(&credential.access_token),
        )
    }

    fn put_with_auth(
        &self,
        path: &str,
        credential: &ManagementCredential,
    ) -> reqwest::blocking::RequestBuilder {
        self.apply_common_headers(
            self.client
                .put(self.url(path))
                .bearer_auth(&credential.access_token),
        )
    }

    fn delete_with_auth(
        &self,
        path: &str,
        credential: &ManagementCredential,
    ) -> reqwest::blocking::RequestBuilder {
        self.apply_common_headers(
            self.client
                .delete(self.url(path))
                .bearer_auth(&credential.access_token),
        )
    }
}

impl ManagementApiClient for HttpManagementApiClient {
    fn start_device_login(&mut self) -> CoreResult<DeviceLoginChallenge> {
        let response = self
            .apply_common_headers(self.client.post(self.url("/auth/device/start")))
            .json(&DeviceLoginStartRequest {
                client_name: "loomex-cli".to_string(),
            })
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        parse_json_response(response)
    }

    fn poll_device_token(&mut self, device_code: &str) -> CoreResult<Option<AuthTokenResponse>> {
        let response = self
            .apply_common_headers(self.client.post(self.url("/auth/device/token")))
            .json(&serde_json::json!({ "device_code": device_code }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        if response.status() == StatusCode::ACCEPTED {
            return Ok(None);
        }
        parse_json_response(response).map(Some)
    }

    fn exchange_api_key(
        &mut self,
        api_key: &str,
        api_secret: &str,
        organization_id: &str,
    ) -> CoreResult<ApiKeyExchangeResult> {
        let response = self
            .apply_common_headers(self.client.post(self.url(RUNNER_AUTH_EXCHANGE_PATH)))
            .json(&serde_json::json!({
                "api_key": api_key,
                "api_secret": api_secret,
                "organization_id": organization_id,
                "runnerName": "Local runner"
            }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerAuthExchangeData> = parse_json_response(response)?;
        let project_id = envelope.data.project_id.or(envelope.data.runner.project_id);
        Ok(ApiKeyExchangeResult {
            token: AuthTokenResponse {
                access_token: envelope.data.runner_token,
                refresh_token: None,
                token_type: envelope.data.token_type,
                expires_at: RUNNER_TOKEN_NON_EXPIRING_EXPIRES_AT.to_string(),
            },
            organization_id: Some(envelope.data.organization_id),
            project_id: project_id.clone(),
            runner_id: Some(envelope.data.runner.id.clone()),
            binding_id: project_id.map(|_| envelope.data.runner.id),
        })
    }

    fn login_workspace(&mut self, email: &str, password: &str) -> CoreResult<WorkspaceLoginResult> {
        let response = self
            .apply_common_headers(self.client.post(self.url("/workspace/auth/login/")))
            .json(&serde_json::json!({
                "email": email,
                "password": password,
            }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<WorkspaceLoginData> = parse_json_response(response)?;
        let organization_id = envelope.data.organization.map(|item| item.id);
        let project_id = envelope
            .data
            .projects
            .into_iter()
            .find(|project| {
                organization_id
                    .as_deref()
                    .map(|org| project.organization_id.as_deref() == Some(org))
                    .unwrap_or(true)
            })
            .map(|project| project.id);
        Ok(WorkspaceLoginResult {
            token: envelope.data.token,
            organization_id,
            project_id,
        })
    }

    fn bootstrap_runner_with_workspace_token(
        &mut self,
        workspace_token: &str,
        organization_id: &str,
        project_id: Option<&str>,
        workspace_root: Option<&str>,
    ) -> CoreResult<ApiKeyExchangeResult> {
        let response = self
            .apply_common_headers(
                self.client
                    .post(self.url(RUNNER_AUTH_BOOTSTRAP_PATH))
                    .bearer_auth(workspace_token),
            )
            .json(&serde_json::json!({
                "organizationId": organization_id,
                "projectId": project_id,
                "workspaceRoot": workspace_root.unwrap_or(""),
                "runnerName": "Local runner",
            }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerAuthExchangeData> = parse_json_response(response)?;
        let project_id = envelope.data.project_id.or(envelope.data.runner.project_id);
        Ok(ApiKeyExchangeResult {
            token: AuthTokenResponse {
                access_token: envelope.data.runner_token,
                refresh_token: None,
                token_type: envelope.data.token_type,
                expires_at: RUNNER_TOKEN_NON_EXPIRING_EXPIRES_AT.to_string(),
            },
            organization_id: Some(envelope.data.organization_id),
            project_id: project_id.clone(),
            runner_id: Some(envelope.data.runner.id.clone()),
            binding_id: project_id.map(|_| envelope.data.runner.id),
        })
    }

    fn list_organizations(
        &mut self,
        credential: &ManagementCredential,
    ) -> CoreResult<Vec<Organization>> {
        let response = self
            .get_with_auth("/organizations", credential)
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let list: OrganizationList = parse_json_response(response)?;
        Ok(list.items)
    }

    fn list_projects(
        &mut self,
        credential: &ManagementCredential,
        organization_id: &str,
    ) -> CoreResult<Vec<Project>> {
        let response = self
            .get_with_auth(
                &format!(
                    "/projects?organization_id={}",
                    encode_query(organization_id)
                ),
                credential,
            )
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let list: ProjectList = parse_json_response(response)?;
        Ok(list.items)
    }

    fn get_project(
        &mut self,
        credential: &ManagementCredential,
        project_id: &str,
    ) -> CoreResult<Project> {
        let response = self
            .get_with_auth(
                &format!("/projects/{}", encode_path(project_id)),
                credential,
            )
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        parse_json_response(response)
    }

    fn get_current_runner(
        &mut self,
        credential: &ManagementCredential,
        organization_id: &str,
    ) -> CoreResult<Runner> {
        let response = self
            .get_with_auth(
                &format!(
                    "/runners/current?organization_id={}",
                    encode_query(organization_id)
                ),
                credential,
            )
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        parse_json_response(response)
    }

    fn upsert_current_runner(
        &mut self,
        credential: &ManagementCredential,
        request: &RunnerUpsertRequest,
        idempotency_key: &str,
    ) -> CoreResult<Runner> {
        let response = self
            .put_with_auth("/runners/current", credential)
            .header("Idempotency-Key", idempotency_key)
            .json(request)
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        parse_json_response(response)
    }

    fn create_project_runner_binding(
        &mut self,
        credential: &ManagementCredential,
        project_id: &str,
        request: &ProjectRunnerBindingCreateRequest,
        idempotency_key: &str,
    ) -> CoreResult<ManagementProjectRunnerBinding> {
        let response = self
            .post_with_auth(
                &format!("/projects/{}/runner-bindings", encode_path(project_id)),
                credential,
            )
            .header("Idempotency-Key", idempotency_key)
            .json(request)
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        parse_json_response(response)
    }

    fn list_project_runner_bindings(
        &mut self,
        credential: &ManagementCredential,
        project_id: &str,
    ) -> CoreResult<Vec<ManagementProjectRunnerBinding>> {
        let response = self
            .get_with_auth(
                &format!("/projects/{}/runner-bindings", encode_path(project_id)),
                credential,
            )
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let list: ProjectRunnerBindingList = parse_json_response(response)?;
        Ok(list.items)
    }

    fn revoke_project_runner_binding(
        &mut self,
        credential: &ManagementCredential,
        project_id: &str,
        binding_id: &str,
        idempotency_key: &str,
    ) -> CoreResult<()> {
        let response = self
            .delete_with_auth(
                &format!(
                    "/projects/{}/runner-bindings/{}",
                    encode_path(project_id),
                    encode_path(binding_id)
                ),
                credential,
            )
            .header("Idempotency-Key", idempotency_key)
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        parse_empty_response(response)
    }

    fn start_workflow_run(
        &mut self,
        credential: &ManagementCredential,
        request: &WorkflowRunStartRequest,
    ) -> CoreResult<WorkflowRunStartResponse> {
        let response = self
            .post_with_auth(
                &format!(
                    "/client/workflows/{}/runs/",
                    encode_path(&request.workflow_id)
                ),
                credential,
            )
            .json(&ClientWorkflowRunStartRequest {
                input: request.inputs.clone(),
                project_runner_binding_id: request.project_runner_binding_id.clone(),
            })
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<WorkflowRunStartResponse> = parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn list_runner_workflows(
        &mut self,
        credential: &ManagementCredential,
    ) -> CoreResult<Vec<RunnerWorkflowSummary>> {
        let response = self
            .get_with_auth("/runner-control/runner/v1/workflows/", credential)
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerWorkflowListResponse> = parse_json_response(response)?;
        Ok(envelope.data.workflows)
    }

    fn start_runner_workflow_execution(
        &mut self,
        credential: &ManagementCredential,
        workflow_id: &str,
        inputs: Value,
        session_id: Option<&str>,
        version: Option<&str>,
    ) -> CoreResult<RunnerWorkflowExecutionResponse> {
        let response = self
            .post_with_auth(
                &format!(
                    "/runner-control/runner/v1/workflows/{}/executions/",
                    encode_path(workflow_id)
                ),
                credential,
            )
            .json(&RunnerWorkflowExecutionStartRequest {
                inputs,
                session_id: session_id.map(str::to_string),
                version: version.map(str::to_string),
            })
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerWorkflowExecutionResponse> =
            parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn get_runner_workflow_execution(
        &mut self,
        credential: &ManagementCredential,
        execution_id: &str,
    ) -> CoreResult<RunnerWorkflowExecutionResponse> {
        let response = self
            .get_with_auth(
                &format!(
                    "/runner-control/runner/v1/executions/{}/",
                    encode_path(execution_id)
                ),
                credential,
            )
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerWorkflowExecutionResponse> =
            parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn list_runner_workflow_executions(
        &mut self,
        credential: &ManagementCredential,
        workflow_id: &str,
        limit: usize,
    ) -> CoreResult<RunnerWorkflowExecutionListResponse> {
        let response = self
            .get_with_auth(
                &format!(
                    "/runner-control/runner/v1/workflows/{}/executions/?limit={}",
                    encode_path(workflow_id),
                    limit.clamp(1, 50)
                ),
                credential,
            )
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerWorkflowExecutionListResponse> =
            parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn get_runner_workflow_input_schema(
        &mut self,
        credential: &ManagementCredential,
        workflow_id: &str,
        version: Option<&str>,
    ) -> CoreResult<RunnerWorkflowInputSchemaResponse> {
        let mut path = format!(
            "/runner-control/runner/v1/workflows/{}/",
            encode_path(workflow_id)
        );
        if let Some(version) = version.filter(|value| !value.trim().is_empty()) {
            path.push_str("?version=");
            path.push_str(&encode_query(version.trim()));
        }
        let response = self
            .get_with_auth(&path, credential)
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerWorkflowInputSchemaResponse> =
            parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn get_workflow_input_schema(
        &mut self,
        credential: &ManagementCredential,
        workflow_id: &str,
    ) -> CoreResult<Option<Value>> {
        let response = self
            .get_with_auth(
                &format!("/client/workflows/{}/", encode_path(workflow_id)),
                credential,
            )
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<ClientWorkflowDetailResponse> = parse_json_response(response)?;
        Ok(envelope
            .data
            .active_version
            .and_then(|version| version.definition.get("inputSchema").cloned())
            .filter(|schema| schema.is_object()))
    }

    fn list_human_requests(
        &mut self,
        credential: &ManagementCredential,
        workflow_id: &str,
        execution_id: Option<&str>,
    ) -> CoreResult<Vec<HumanRequestSummary>> {
        if let Some(execution_id) = execution_id.filter(|value| !value.trim().is_empty()) {
            let response = self
                .get_with_auth(
                    &format!(
                        "/runner-control/runner/v1/executions/{}/human-requests/?status=pending",
                        encode_path(execution_id.trim())
                    ),
                    credential,
                )
                .send()
                .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
            let envelope: ClientEnvelope<RunnerHumanRequestListResponse> =
                parse_json_response(response)?;
            return Ok(envelope.data.human_requests);
        }
        let query = format!("workflow_id={}", encode_query(workflow_id));
        let response = self
            .get_with_auth(&format!("/client/human-inbox/?{query}"), credential)
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<Vec<HumanRequestSummary>> = parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn resolve_human_request(
        &mut self,
        credential: &ManagementCredential,
        request_id: &str,
        payload: &Value,
    ) -> CoreResult<HumanRequestResolveResponse> {
        let response = self
            .post_with_auth(
                &format!(
                    "/runner-control/runner/v1/human-requests/{}/resolve/",
                    encode_path(request_id)
                ),
                credential,
            )
            .json(payload)
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<HumanRequestResolveResponse> = parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn create_runner_session(
        &mut self,
        credential: &ManagementCredential,
        workspace_root: &str,
        manifest: Value,
        transport: &str,
    ) -> CoreResult<RunnerSessionResponse> {
        let response = self
            .post_with_auth("/runner-control/runner/v1/sessions/", credential)
            .json(&serde_json::json!({
                "workspaceRoot": workspace_root,
                "manifest": manifest,
                "transport": transport,
            }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerSessionResponse> = parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn heartbeat_runner_session(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
        manifest: Value,
    ) -> CoreResult<RunnerSessionResponse> {
        let response = self
            .post_with_auth(
                &format!(
                    "/runner-control/runner/v1/sessions/{}/heartbeat/",
                    encode_path(session_id)
                ),
                credential,
            )
            .json(&serde_json::json!({ "manifest": manifest }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerSessionResponse> = parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn lease_runner_job(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
    ) -> CoreResult<RunnerJobResponse> {
        let response = self
            .post_with_auth("/runner-control/runner/v1/jobs/lease/", credential)
            .json(&serde_json::json!({ "sessionId": session_id }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerJobResponse> = parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn start_runner_job(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
        job_id: &str,
    ) -> CoreResult<RunnerJobResponse> {
        let response = self
            .post_with_auth(
                &format!(
                    "/runner-control/runner/v1/jobs/{}/start/",
                    encode_path(job_id)
                ),
                credential,
            )
            .json(&serde_json::json!({ "sessionId": session_id }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerJobResponse> = parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn append_runner_job_events(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
        job_id: &str,
        events: Vec<Value>,
    ) -> CoreResult<RunnerJobEventCreateResponse> {
        let response = self
            .post_with_auth(
                &format!(
                    "/runner-control/runner/v1/jobs/{}/events/",
                    encode_path(job_id)
                ),
                credential,
            )
            .json(&serde_json::json!({ "sessionId": session_id, "events": events }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerJobEventCreateResponse> = parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn complete_runner_job(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
        job_id: &str,
        result: Value,
    ) -> CoreResult<RunnerJobResponse> {
        let response = self
            .post_with_auth(
                &format!(
                    "/runner-control/runner/v1/jobs/{}/complete/",
                    encode_path(job_id)
                ),
                credential,
            )
            .json(&serde_json::json!({ "sessionId": session_id, "result": result }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerJobResponse> = parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn fail_runner_job(
        &mut self,
        credential: &ManagementCredential,
        session_id: &str,
        job_id: &str,
        error: Value,
    ) -> CoreResult<RunnerJobResponse> {
        let response = self
            .post_with_auth(
                &format!(
                    "/runner-control/runner/v1/jobs/{}/fail/",
                    encode_path(job_id)
                ),
                credential,
            )
            .json(&serde_json::json!({ "sessionId": session_id, "error": error }))
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        let envelope: ClientEnvelope<RunnerJobResponse> = parse_json_response(response)?;
        Ok(envelope.data)
    }

    fn issue_stream_credential(
        &mut self,
        credential: &ManagementCredential,
        request: &StreamCredentialRequest,
        idempotency_key: &str,
    ) -> CoreResult<StreamCredentialResponse> {
        let response = self
            .post_with_auth("/runners/current/stream-credential", credential)
            .header("Idempotency-Key", idempotency_key)
            .json(request)
            .send()
            .map_err(|err| CoreError::new("MANAGEMENT_HTTP_FAILED", err.to_string()))?;
        parse_json_response(response)
    }
}

fn parse_json_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::blocking::Response,
) -> CoreResult<T> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(management_error_from_status_and_body(
            status.as_u16(),
            &body,
        ));
    }
    response
        .json::<T>()
        .map_err(|err| CoreError::new("MANAGEMENT_RESPONSE_INVALID", err.to_string()))
}

fn parse_empty_response(response: reqwest::blocking::Response) -> CoreResult<()> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(management_error_from_status_and_body(
            status.as_u16(),
            &body,
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    code: String,
    message: String,
    #[serde(default)]
    request_id: String,
    details: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelopeResponse {
    error: ErrorResponse,
}

fn management_error_from_status_and_body(status: u16, body: &str) -> CoreError {
    let parsed = serde_json::from_str::<ErrorResponse>(body)
        .ok()
        .or_else(|| {
            serde_json::from_str::<ErrorEnvelopeResponse>(body)
                .ok()
                .map(|envelope| envelope.error)
        });
    if let Some(error) = parsed {
        let code: &'static str = Box::leak(error.code.into_boxed_str());
        let mut message = error.message;
        if !error.request_id.is_empty() {
            message.push_str(&format!(" request_id={}", error.request_id));
        }
        if let Some(details) = error.details {
            message.push_str(&format!(" details={details}"));
        }
        return CoreError::new(code, message);
    }
    CoreError::new(
        match status {
            401 => "MANAGEMENT_AUTH_FAILED",
            403 => "MANAGEMENT_PERMISSION_DENIED",
            _ => "MANAGEMENT_HTTP_STATUS",
        },
        format!("management API returned HTTP {status}"),
    )
}

pub fn parse_rfc3339_utc_epoch_seconds(value: &str) -> CoreResult<u64> {
    let Some((date, time)) = value.strip_suffix('Z').and_then(|v| v.split_once('T')) else {
        return Err(CoreError::new(
            "AUTH_TOKEN_EXPIRY_INVALID",
            "expires_at must be an RFC3339 UTC timestamp",
        ));
    };
    let mut date_parts = date.split('-');
    let year = parse_i64(date_parts.next(), "year")?;
    let month = parse_i64(date_parts.next(), "month")?;
    let day = parse_i64(date_parts.next(), "day")?;
    if date_parts.next().is_some() {
        return Err(CoreError::new("AUTH_TOKEN_EXPIRY_INVALID", "invalid date"));
    }
    let mut time_parts = time.split(':');
    let hour = parse_i64(time_parts.next(), "hour")?;
    let minute = parse_i64(time_parts.next(), "minute")?;
    let second = parse_i64(time_parts.next(), "second")?;
    if time_parts.next().is_some() {
        return Err(CoreError::new("AUTH_TOKEN_EXPIRY_INVALID", "invalid time"));
    }
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return Err(CoreError::new(
            "AUTH_TOKEN_EXPIRY_INVALID",
            "expires_at contains an out-of-range timestamp component",
        ));
    }
    let days = days_from_civil(year, month, day)?;
    let seconds = days
        .checked_mul(86_400)
        .and_then(|value| value.checked_add(hour * 3_600 + minute * 60 + second))
        .ok_or_else(|| CoreError::new("AUTH_TOKEN_EXPIRY_INVALID", "timestamp overflow"))?;
    u64::try_from(seconds)
        .map_err(|_| CoreError::new("AUTH_TOKEN_EXPIRY_INVALID", "timestamp is before epoch"))
}

fn parse_i64(value: Option<&str>, field: &'static str) -> CoreResult<i64> {
    value
        .ok_or_else(|| CoreError::new("AUTH_TOKEN_EXPIRY_INVALID", field))?
        .parse::<i64>()
        .map_err(|_| CoreError::new("AUTH_TOKEN_EXPIRY_INVALID", field))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> CoreResult<i64> {
    let adjusted_year = year - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    if !(0..=365).contains(&day_of_year) {
        return Err(CoreError::new("AUTH_TOKEN_EXPIRY_INVALID", "invalid date"));
    }
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Ok(era * 146_097 + day_of_era - 719_468)
}

fn encode_path(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn encode_query(value: &str) -> String {
    encode_path(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_exchange_uses_runner_control_endpoint() {
        let client = HttpManagementApiClient::new("http://loomex.localhost:28080", None).unwrap();

        assert_eq!(
            "http://loomex.localhost:28080/api/v1/runner-control/runner/v1/auth/exchange/",
            client.url(RUNNER_AUTH_EXCHANGE_PATH)
        );
    }

    #[test]
    fn local_store_does_not_write_plain_token_and_round_trips() {
        let root = std::env::temp_dir().join(format!(
            "loomex-credentials-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        let store = LocalCredentialStore::new(root.clone());
        let credential = ManagementCredential::from_token_response(
            "default",
            "org_123",
            AuthTokenResponse {
                access_token: "management_secret".to_string(),
                refresh_token: Some("refresh_secret".to_string()),
                token_type: "Bearer".to_string(),
                expires_at: "2026-06-29T00:00:00Z".to_string(),
            },
            CredentialStorageBackend::LocalFileFallback,
        )
        .unwrap();

        store.save(&credential).unwrap();
        let raw = fs::read_to_string(root.join("default.json")).unwrap();
        let loaded = store.load("default").unwrap().unwrap();
        let _ = fs::remove_dir_all(&root);

        assert!(!raw.contains("management_secret"));
        assert!(!raw.contains("refresh_secret"));
        assert_eq!(credential.access_token, loaded.access_token);
        assert_eq!(credential.refresh_token, loaded.refresh_token);
        assert!(loaded.storage_warning.unwrap().contains("fallback"));
    }

    #[test]
    fn credential_debug_redacts_token() {
        let credential = ManagementCredential::from_token_response(
            "default",
            "org_123",
            AuthTokenResponse {
                access_token: "management_secret".to_string(),
                refresh_token: Some("refresh_secret".to_string()),
                token_type: "Bearer".to_string(),
                expires_at: "2026-06-29T00:00:00Z".to_string(),
            },
            CredentialStorageBackend::LocalFileFallback,
        )
        .unwrap();

        let debug = format!("{credential:?}");

        assert!(!debug.contains("management_secret"));
        assert!(!debug.contains("refresh_secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn invalid_auth_token_shape_is_rejected() {
        let err = ManagementCredential::from_token_response(
            "default",
            "org_123",
            AuthTokenResponse {
                access_token: String::new(),
                refresh_token: None,
                token_type: "Bearer".to_string(),
                expires_at: "2026-06-29T00:00:00Z".to_string(),
            },
            CredentialStorageBackend::LocalFileFallback,
        )
        .unwrap_err();

        assert_eq!("AUTH_TOKEN_INVALID", err.code);
    }

    #[test]
    fn expired_and_near_expiry_management_tokens_fail_deterministically() {
        let credential = ManagementCredential::from_token_response(
            "default",
            "org_123",
            AuthTokenResponse {
                access_token: "management_secret".to_string(),
                refresh_token: Some("refresh_secret".to_string()),
                token_type: "Bearer".to_string(),
                expires_at: "2026-06-29T00:00:00Z".to_string(),
            },
            CredentialStorageBackend::LocalFileFallback,
        )
        .unwrap();

        assert_eq!(
            "AUTH_TOKEN_EXPIRED",
            credential
                .validate_not_expiring(1_782_691_200, 300)
                .unwrap_err()
                .code
        );
        assert_eq!(
            "AUTH_TOKEN_EXPIRED",
            credential
                .validate_not_expiring(1_782_690_950, 300)
                .unwrap_err()
                .code
        );
        credential
            .validate_not_expiring(1_782_690_000, 300)
            .unwrap();
    }

    #[test]
    fn management_error_envelope_preserves_code_message_and_request_id() {
        let err = management_error_from_status_and_body(
            403,
            r#"{"code":"PROJECT_FORBIDDEN","message":"No access","details":{"project_id":"prj_123"},"request_id":"req_123"}"#,
        );

        assert_eq!("PROJECT_FORBIDDEN", err.code);
        assert!(err.message.contains("No access"));
        assert!(err.message.contains("request_id=req_123"));
        assert!(err.message.contains("project_id"));
    }

    #[test]
    fn runner_workflow_execution_response_preserves_ai_trace() {
        let response = serde_json::from_str::<RunnerWorkflowExecutionResponse>(
            r#"{
                "execution": {"id": "exec_123", "status": "running"},
                "aiTrace": {
                    "schemaVersion": "loomex.runner.aiTrace/v1",
                    "events": [{"sequence": 1, "type": "ai.message.completed", "content": "done"}]
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            response
                .ai_trace
                .as_ref()
                .and_then(|trace| trace.get("events"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn unauthorized_error_envelope_preserves_contract_code() {
        let err = management_error_from_status_and_body(
            401,
            r#"{"code":"AUTH_TOKEN_EXPIRED","message":"Token expired","request_id":"req_auth","details":{"profile":"default"}}"#,
        );

        assert_eq!("AUTH_TOKEN_EXPIRED", err.code);
        assert!(err.message.contains("Token expired"));
        assert!(err.message.contains("request_id=req_auth"));
    }

    #[test]
    fn nested_management_error_envelope_preserves_contract_code() {
        let err = management_error_from_status_and_body(
            422,
            r#"{"error":{"code":"LOCAL_RUNNER_REQUIRED","message":"Local workflow execution requires an online project runner.","details":{}},"meta":{"correlationId":"req_nested","version":"v1"}}"#,
        );

        assert_eq!("LOCAL_RUNNER_REQUIRED", err.code);
        assert!(err
            .message
            .contains("Local workflow execution requires an online project runner."));
        assert!(!err.message.contains("management API returned HTTP 422"));
    }
}
