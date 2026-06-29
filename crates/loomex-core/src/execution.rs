use crate::binding::{
    validate_session_and_grant_for_local_tool_call, BindingValidationContext, ProjectRunnerBinding,
    RunnerCapabilityGrant, RunnerSession,
};
use crate::capability::{CapabilityExecutor, CapabilityRequest, CapabilityResult};
use crate::policy::{enforce_policy_decision, PolicyEngine, PolicyEvaluationInput};
use crate::{CoreError, CoreResult};

pub struct ExecutionRegistry {
    executors: Vec<Box<dyn CapabilityExecutor>>,
}

impl ExecutionRegistry {
    pub fn new(executors: Vec<Box<dyn CapabilityExecutor>>) -> Self {
        Self { executors }
    }

    pub fn execute(&self, request: CapabilityRequest) -> CoreResult<CapabilityResult> {
        let executor = self
            .executors
            .iter()
            .find(|candidate| candidate.supports(&request.capability))
            .ok_or_else(|| {
                CoreError::new("CAPABILITY_NOT_REGISTERED", request.capability.clone())
            })?;
        executor.execute(request)
    }

    pub fn execute_with_binding(
        &self,
        request: CapabilityRequest,
        binding: Option<&ProjectRunnerBinding>,
        session: Option<&RunnerSession>,
        grant: Option<&RunnerCapabilityGrant>,
        context: &BindingValidationContext,
        requested_path: &str,
        resolved_path: Option<&str>,
    ) -> CoreResult<CapabilityResult> {
        validate_session_and_grant_for_local_tool_call(
            binding,
            session,
            grant,
            context,
            &request.capability,
            requested_path,
            resolved_path,
        )?;
        self.execute(request)
    }

    pub fn execute_with_policy(
        &self,
        request: CapabilityRequest,
        binding: Option<&ProjectRunnerBinding>,
        session: Option<&RunnerSession>,
        grant: Option<&RunnerCapabilityGrant>,
        context: &BindingValidationContext,
        policy_engine: &PolicyEngine,
        policy_input: &PolicyEvaluationInput,
        requested_path: &str,
        resolved_path: Option<&str>,
    ) -> CoreResult<CapabilityResult> {
        validate_policy_input_matches_request(&request, policy_input, requested_path)?;
        validate_session_and_grant_for_local_tool_call(
            binding,
            session,
            grant,
            context,
            &request.capability,
            requested_path,
            resolved_path,
        )?;
        let binding = binding.ok_or_else(|| {
            CoreError::new(
                "PROJECT_BINDING_REQUIRED",
                "local tool call requires an active project runner binding",
            )
        })?;
        let evaluation = policy_engine.dry_run(policy_input, binding)?;
        enforce_policy_decision(&evaluation)?;
        self.execute(request)
    }
}

fn validate_policy_input_matches_request(
    request: &CapabilityRequest,
    policy_input: &PolicyEvaluationInput,
    requested_path: &str,
) -> CoreResult<()> {
    if policy_input.capability != request.capability {
        return Err(CoreError::new(
            "POLICY_CAPABILITY_MISMATCH",
            "policy input capability must match execution request capability",
        ));
    }
    let Some(policy_path) = &policy_input.requested_path else {
        return Err(CoreError::new(
            "POLICY_PATH_MISMATCH",
            "policy input path is required for path-bound execution",
        ));
    };
    if policy_path != requested_path {
        return Err(CoreError::new(
            "POLICY_PATH_MISMATCH",
            "policy input path must match execution request path",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::{
        BindingRegistry, CreateBindingInput, RunnerCapabilityGrant, RunnerSession,
        RunnerSessionStatus,
    };
    use crate::policy::{
        PolicyDecision, PolicyEngine, PolicyEvaluationInput, PolicyLayer, PolicyRule, PolicySource,
    };

    struct EchoExecutor;

    impl CapabilityExecutor for EchoExecutor {
        fn capability(&self) -> &'static str {
            "mock.echo"
        }

        fn execute(&self, request: CapabilityRequest) -> CoreResult<CapabilityResult> {
            Ok(CapabilityResult {
                capability: request.capability,
                output: request.input,
            })
        }
    }

    fn registry_with_binding() -> (BindingRegistry, ProjectRunnerBinding) {
        let mut registry = BindingRegistry::default();
        let binding = registry
            .create_or_reuse_binding(CreateBindingInput {
                organization_id: "org_123".to_string(),
                project_id: "prj_123".to_string(),
                runner_device_id: "device_123".to_string(),
                local_workspace_root: "/Users/example/app".to_string(),
                local_root_fingerprint: None,
                created_by: "user_123".to_string(),
                now_epoch_ms: 1_000,
                allow_same_path_different_project: false,
                project_archived: false,
                runner_disabled: false,
            })
            .unwrap();
        (registry, binding)
    }

    fn context() -> BindingValidationContext {
        BindingValidationContext {
            organization_id: "org_123".to_string(),
            project_id: "prj_123".to_string(),
            project_permission_active: true,
        }
    }

    fn session(binding: &ProjectRunnerBinding) -> RunnerSession {
        RunnerSession {
            id: "session_123".to_string(),
            organization_id: binding.organization_id.clone(),
            project_id: binding.project_id.clone(),
            runner_device_id: binding.runner_device_id.clone(),
            project_runner_binding_id: binding.id.clone(),
            status: RunnerSessionStatus::Connected,
            last_seen_at_epoch_ms: Some(1_000),
            lease_expires_at_epoch_ms: 31_000,
            connected_at_epoch_ms: 1_000,
            disconnected_at_epoch_ms: None,
            replaced_by_session_id: None,
            disconnect_reason: None,
        }
    }

    fn grant(binding: &ProjectRunnerBinding, capability: &str) -> RunnerCapabilityGrant {
        RunnerCapabilityGrant {
            project_runner_binding_id: binding.id.clone(),
            capability: capability.to_string(),
            granted_by: "policy_123".to_string(),
            created_at_epoch_ms: 1_000,
            revoked_at_epoch_ms: None,
        }
    }

    fn policy_input(capability: &str, requested_path: &str) -> PolicyEvaluationInput {
        let mut input = PolicyEvaluationInput::capability(capability);
        input.requested_path = Some(requested_path.to_string());
        input
    }

    #[test]
    fn execute_with_binding_runs_inside_active_binding() {
        let (_, binding) = registry_with_binding();
        let registry = ExecutionRegistry::new(vec![Box::new(EchoExecutor)]);

        let result = registry
            .execute_with_binding(
                CapabilityRequest {
                    capability: "mock.echo".to_string(),
                    input: "ok".to_string(),
                },
                Some(&binding),
                Some(&session(&binding)),
                Some(&grant(&binding, "mock.echo")),
                &context(),
                "/Users/example/app/file.txt",
                None,
            )
            .unwrap();

        assert_eq!("ok", result.output);
    }

    #[test]
    fn execute_with_binding_blocks_missing_binding_before_local_tool_call() {
        let registry = ExecutionRegistry::new(vec![Box::new(EchoExecutor)]);

        let err = registry
            .execute_with_binding(
                CapabilityRequest {
                    capability: "mock.echo".to_string(),
                    input: "ok".to_string(),
                },
                None,
                None,
                None,
                &context(),
                "/Users/example/app/file.txt",
                None,
            )
            .unwrap_err();

        assert_eq!("PROJECT_BINDING_REQUIRED", err.code);
    }

    #[test]
    fn execute_with_binding_blocks_revoked_grant_before_executor() {
        let (_, binding) = registry_with_binding();
        let registry = ExecutionRegistry::new(vec![Box::new(EchoExecutor)]);
        let mut grant = grant(&binding, "mock.echo");
        grant.revoked_at_epoch_ms = Some(2_000);

        let err = registry
            .execute_with_binding(
                CapabilityRequest {
                    capability: "mock.echo".to_string(),
                    input: "ok".to_string(),
                },
                Some(&binding),
                Some(&session(&binding)),
                Some(&grant),
                &context(),
                "/Users/example/app/file.txt",
                None,
            )
            .unwrap_err();

        assert_eq!("RUNNER_CAPABILITY_GRANT_REVOKED", err.code);
    }

    #[test]
    fn execute_with_policy_requires_allow_decision_before_executor() {
        let (_, binding) = registry_with_binding();
        let registry = ExecutionRegistry::new(vec![Box::new(EchoExecutor)]);
        let engine = PolicyEngine::new(vec![PolicyLayer {
            source: PolicySource::Project,
            default_decision: None,
            rules: vec![PolicyRule::for_capability(
                "mock.echo",
                PolicyDecision::Allow,
            )],
        }]);

        let result = registry
            .execute_with_policy(
                CapabilityRequest {
                    capability: "mock.echo".to_string(),
                    input: "ok".to_string(),
                },
                Some(&binding),
                Some(&session(&binding)),
                Some(&grant(&binding, "mock.echo")),
                &context(),
                &engine,
                &policy_input("mock.echo", "/Users/example/app/file.txt"),
                "/Users/example/app/file.txt",
                None,
            )
            .unwrap();

        assert_eq!("ok", result.output);
    }

    #[test]
    fn execute_with_policy_blocks_ask_decision_before_executor() {
        let (_, binding) = registry_with_binding();
        let registry = ExecutionRegistry::new(vec![Box::new(EchoExecutor)]);
        let engine = PolicyEngine::default();

        let err = registry
            .execute_with_policy(
                CapabilityRequest {
                    capability: "mock.echo".to_string(),
                    input: "ok".to_string(),
                },
                Some(&binding),
                Some(&session(&binding)),
                Some(&grant(&binding, "mock.echo")),
                &context(),
                &engine,
                &policy_input("mock.echo", "/Users/example/app/file.txt"),
                "/Users/example/app/file.txt",
                None,
            )
            .unwrap_err();

        assert_eq!("POLICY_APPROVAL_REQUIRED", err.code);
    }

    #[test]
    fn execute_with_policy_rejects_mismatched_policy_capability_before_executor() {
        let (_, binding) = registry_with_binding();
        let registry = ExecutionRegistry::new(vec![Box::new(EchoExecutor)]);
        let engine = PolicyEngine::new(vec![PolicyLayer {
            source: PolicySource::Project,
            default_decision: None,
            rules: vec![PolicyRule::for_capability(
                "git.status",
                PolicyDecision::Allow,
            )],
        }]);

        let err = registry
            .execute_with_policy(
                CapabilityRequest {
                    capability: "mock.echo".to_string(),
                    input: "ok".to_string(),
                },
                Some(&binding),
                Some(&session(&binding)),
                Some(&grant(&binding, "mock.echo")),
                &context(),
                &engine,
                &policy_input("git.status", "/Users/example/app/file.txt"),
                "/Users/example/app/file.txt",
                None,
            )
            .unwrap_err();

        assert_eq!("POLICY_CAPABILITY_MISMATCH", err.code);
    }

    #[test]
    fn execute_with_policy_rejects_missing_policy_path_before_executor() {
        let (_, binding) = registry_with_binding();
        let registry = ExecutionRegistry::new(vec![Box::new(EchoExecutor)]);
        let mut sensitive_rule = PolicyRule::for_capability("mock.echo", PolicyDecision::Deny);
        sensitive_rule.sensitive_path_pattern = Some(".env".to_string());
        let engine = PolicyEngine::new(vec![PolicyLayer {
            source: PolicySource::Project,
            default_decision: Some(PolicyDecision::Allow),
            rules: vec![sensitive_rule],
        }]);

        let err = registry
            .execute_with_policy(
                CapabilityRequest {
                    capability: "mock.echo".to_string(),
                    input: "ok".to_string(),
                },
                Some(&binding),
                Some(&session(&binding)),
                Some(&grant(&binding, "mock.echo")),
                &context(),
                &engine,
                &PolicyEvaluationInput::capability("mock.echo"),
                "/Users/example/app/.env",
                None,
            )
            .unwrap_err();

        assert_eq!("POLICY_PATH_MISMATCH", err.code);
    }
}
