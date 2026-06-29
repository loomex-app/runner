use std::path::{Path, PathBuf};

use crate::{CoreError, CoreResult};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RunnerServicePlatform {
    LinuxUserSystemd,
    LinuxSystemSystemd,
    WindowsService,
}

impl RunnerServicePlatform {
    pub fn parse(value: &str) -> CoreResult<Self> {
        match value {
            "linux-user" | "systemd-user" => Ok(Self::LinuxUserSystemd),
            "linux-system" | "systemd-system" => Ok(Self::LinuxSystemSystemd),
            "windows" | "windows-service" => Ok(Self::WindowsService),
            _ => Err(CoreError::new(
                "RUNNER_SERVICE_PLATFORM_UNSUPPORTED",
                format!("unsupported service platform {value}"),
            )),
        }
    }

    pub fn current() -> Self {
        if cfg!(windows) {
            Self::WindowsService
        } else {
            Self::LinuxUserSystemd
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LinuxUserSystemd => "linux-user",
            Self::LinuxSystemSystemd => "linux-system",
            Self::WindowsService => "windows",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerServiceSpec {
    pub service_name: String,
    pub binary_path: PathBuf,
    pub config_path: PathBuf,
    pub profile: Option<String>,
    pub log_path: Option<PathBuf>,
    pub working_directory: Option<PathBuf>,
}

impl RunnerServiceSpec {
    pub fn validate(&self) -> CoreResult<()> {
        validate_service_name(&self.service_name)?;
        validate_absolute_path(&self.binary_path, "RUNNER_SERVICE_BINARY_INVALID")?;
        validate_absolute_path(&self.config_path, "RUNNER_SERVICE_CONFIG_INVALID")?;
        if let Some(path) = &self.log_path {
            validate_absolute_path(path, "RUNNER_SERVICE_LOG_PATH_INVALID")?;
        }
        if let Some(path) = &self.working_directory {
            validate_absolute_path(path, "RUNNER_SERVICE_WORKDIR_INVALID")?;
        }
        if self
            .profile
            .as_ref()
            .is_some_and(|profile| profile.trim().is_empty())
        {
            return Err(CoreError::new(
                "RUNNER_SERVICE_PROFILE_INVALID",
                "profile cannot be empty",
            ));
        }
        Ok(())
    }

    pub fn render(&self, platform: RunnerServicePlatform) -> CoreResult<RunnerServiceManifest> {
        self.validate()?;
        match platform {
            RunnerServicePlatform::LinuxUserSystemd => Ok(RunnerServiceManifest::systemd_user(
                render_systemd_unit(self, "default.target"),
            )),
            RunnerServicePlatform::LinuxSystemSystemd => Ok(RunnerServiceManifest::systemd_system(
                render_systemd_unit(self, "multi-user.target"),
            )),
            RunnerServicePlatform::WindowsService => Ok(RunnerServiceManifest::windows(
                render_windows_install_script(self),
                render_windows_uninstall_script(&self.service_name),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerServiceManifest {
    pub platform: RunnerServicePlatform,
    pub install_path: String,
    pub uninstall_path: Option<String>,
    pub content: String,
    pub uninstall_content: Option<String>,
}

impl RunnerServiceManifest {
    fn systemd_user(content: String) -> Self {
        Self {
            platform: RunnerServicePlatform::LinuxUserSystemd,
            install_path: "~/.config/systemd/user/loomex-runner.service".to_string(),
            uninstall_path: None,
            content,
            uninstall_content: None,
        }
    }

    fn systemd_system(content: String) -> Self {
        Self {
            platform: RunnerServicePlatform::LinuxSystemSystemd,
            install_path: "/etc/systemd/system/loomex-runner.service".to_string(),
            uninstall_path: None,
            content,
            uninstall_content: None,
        }
    }

    fn windows(install_content: String, uninstall_content: String) -> Self {
        Self {
            platform: RunnerServicePlatform::WindowsService,
            install_path: "loomex-runner-service-install.ps1".to_string(),
            uninstall_path: Some("loomex-runner-service-uninstall.ps1".to_string()),
            content: install_content,
            uninstall_content: Some(uninstall_content),
        }
    }
}

pub fn validate_cross_platform_relative_path(path: &str) -> CoreResult<()> {
    if path.trim().is_empty() || path.contains('\0') || path.contains('\n') || path.contains('\r') {
        return Err(CoreError::new(
            "WORKSPACE_PATH_INVALID",
            "workspace path is invalid",
        ));
    }
    let normalized = path.replace('\\', "/");
    if normalized.starts_with("//") || looks_like_windows_drive_path(path) {
        return Err(CoreError::new(
            "WORKSPACE_PATH_ESCAPE",
            "workspace path escapes the binding root",
        ));
    }
    if normalized.starts_with('/') {
        return Err(CoreError::new(
            "WORKSPACE_PATH_ABSOLUTE",
            "workspace path must be relative",
        ));
    }
    if normalized.contains("/../")
        || normalized == ".."
        || normalized.starts_with("../")
        || normalized.ends_with("/..")
    {
        return Err(CoreError::new(
            "WORKSPACE_PATH_OUTSIDE_ROOT",
            "workspace path escapes the binding root",
        ));
    }
    Ok(())
}

fn render_systemd_unit(spec: &RunnerServiceSpec, wanted_by: &str) -> String {
    let mut args = vec![
        systemd_quote(spec.binary_path.as_path()),
        "runner".to_string(),
        "service".to_string(),
        "run".to_string(),
        "--config".to_string(),
        systemd_quote(spec.config_path.as_path()),
    ];
    if let Some(profile) = &spec.profile {
        args.push("--profile".to_string());
        args.push(systemd_quote_text(profile));
    }
    let working_directory = spec
        .working_directory
        .as_deref()
        .or_else(|| spec.config_path.parent())
        .map(systemd_quote)
        .unwrap_or_else(|| "\"/\"".to_string());
    let mut unit = format!(
        "[Unit]\n\
Description=Loomex Runner\n\
After=network-online.target\n\
Wants=network-online.target\n\
\n\
[Service]\n\
Type=simple\n\
WorkingDirectory={working_directory}\n\
ExecStart={}\n\
Restart=always\n\
RestartSec=5\n\
StandardOutput=journal\n\
StandardError=journal\n",
        args.join(" ")
    );
    if let Some(log_path) = &spec.log_path {
        unit.push_str(&format!(
            "Environment=LOOMEX_RUNNER_LOG_PATH={}\n",
            systemd_quote(log_path)
        ));
    }
    unit.push_str(&format!("\n[Install]\nWantedBy={wanted_by}\n"));
    unit
}

fn render_windows_install_script(spec: &RunnerServiceSpec) -> String {
    let mut command_line = format!(
        "{} runner service run --config {}",
        windows_command_line_quote(&spec.binary_path.to_string_lossy()),
        windows_command_line_quote(&spec.config_path.to_string_lossy())
    );
    if let Some(profile) = &spec.profile {
        command_line.push_str(&format!(
            " --profile {}",
            windows_command_line_quote(profile)
        ));
    }
    if let Some(log_path) = &spec.log_path {
        command_line.push_str(&format!(
            " --log-path {}",
            windows_command_line_quote(&log_path.to_string_lossy())
        ));
    }
    format!(
        "$ErrorActionPreference = 'Stop'\r\n\
$Name = {name}\r\n\
$BinaryPathName = {binary_path_name}\r\n\
if (Get-Service -Name $Name -ErrorAction SilentlyContinue) {{\r\n\
  throw \"RUNNER_SERVICE_ALREADY_INSTALLED: $Name already exists\"\r\n\
}}\r\n\
New-Service -Name $Name -BinaryPathName $BinaryPathName -DisplayName 'Loomex Runner' -StartupType Automatic\r\n\
Start-Service -Name $Name\r\n",
        name = powershell_quote_text(&spec.service_name),
        binary_path_name = powershell_quote_text(&command_line),
    )
}

fn render_windows_uninstall_script(service_name: &str) -> String {
    format!(
        "$ErrorActionPreference = 'Stop'\r\n\
$Name = {name}\r\n\
$Service = Get-Service -Name $Name -ErrorAction SilentlyContinue\r\n\
if ($Service) {{\r\n\
  if ($Service.Status -ne 'Stopped') {{ Stop-Service -Name $Name -Force }}\r\n\
  sc.exe delete $Name | Out-Null\r\n\
}}\r\n",
        name = powershell_quote_text(service_name),
    )
}

fn validate_service_name(value: &str) -> CoreResult<()> {
    if value.trim().is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(CoreError::new(
            "RUNNER_SERVICE_NAME_INVALID",
            "service name must contain only letters, numbers, dot, dash, or underscore",
        ));
    }
    Ok(())
}

fn validate_absolute_path(path: &Path, code: &'static str) -> CoreResult<()> {
    if path.as_os_str().is_empty() || !is_absolute_cross_platform(path) {
        return Err(CoreError::new(code, "path must be absolute"));
    }
    Ok(())
}

fn is_absolute_cross_platform(path: &Path) -> bool {
    path.is_absolute() || path.to_str().is_some_and(looks_like_windows_absolute_path)
}

fn looks_like_windows_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn looks_like_windows_absolute_path(value: &str) -> bool {
    let normalized = value.replace('\\', "/");
    normalized.starts_with("//")
        || (looks_like_windows_drive_path(value)
            && normalized
                .as_bytes()
                .get(2)
                .is_some_and(|byte| *byte == b'/'))
}

fn systemd_quote(path: &Path) -> String {
    systemd_quote_text(&path.to_string_lossy())
}

fn systemd_quote_text(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn powershell_quote_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn windows_command_line_quote(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0usize;
    for ch in value.chars() {
        match ch {
            '\\' => {
                backslashes += 1;
                quoted.push('\\');
            }
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push_str("\\\"");
                backslashes = 0;
            }
            _ => {
                backslashes = 0;
                quoted.push(ch);
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_user_unit_quotes_paths_with_spaces_and_uses_journal() {
        let spec = RunnerServiceSpec {
            service_name: "loomex-runner".to_string(),
            binary_path: PathBuf::from("/opt/Loomex/bin/loomex"),
            config_path: PathBuf::from("/home/dev/My App/.loomex/config.toml"),
            profile: Some("default".to_string()),
            log_path: Some(PathBuf::from("/home/dev/.loomex/runner log.jsonl")),
            working_directory: None,
        };

        let manifest = spec
            .render(RunnerServicePlatform::LinuxUserSystemd)
            .unwrap();

        assert_eq!(RunnerServicePlatform::LinuxUserSystemd, manifest.platform);
        assert!(manifest.content.contains("ExecStart=\"/opt/Loomex/bin/loomex\" runner service run --config \"/home/dev/My App/.loomex/config.toml\" --profile \"default\""));
        assert!(manifest.content.contains("StandardOutput=journal"));
        assert!(manifest
            .content
            .contains("Environment=LOOMEX_RUNNER_LOG_PATH=\"/home/dev/.loomex/runner log.jsonl\""));
        assert!(manifest.content.ends_with("WantedBy=default.target\n"));
    }

    #[test]
    fn windows_manifest_uses_crlf_and_service_commands() {
        let spec = RunnerServiceSpec {
            service_name: "loomex-runner".to_string(),
            binary_path: PathBuf::from("C:\\Program Files\\Loomex\\loomex.exe"),
            config_path: PathBuf::from("C:\\Users\\Dev User\\.loomex\\config.toml"),
            profile: Some("work".to_string()),
            log_path: Some(PathBuf::from("C:\\Users\\Dev User\\.loomex\\runner.log")),
            working_directory: None,
        };

        let manifest = spec.render(RunnerServicePlatform::WindowsService).unwrap();

        assert_eq!(RunnerServicePlatform::WindowsService, manifest.platform);
        assert!(manifest.content.contains("New-Service"));
        assert!(manifest.content.contains("\"C:\\Program Files\\Loomex\\loomex.exe\" runner service run --config \"C:\\Users\\Dev User\\.loomex\\config.toml\""));
        assert!(manifest
            .content
            .contains("--log-path \"C:\\Users\\Dev User\\.loomex\\runner.log\""));
        assert!(manifest.content.contains("--profile"));
        assert!(manifest.content.contains("work"));
        assert!(manifest.content.contains("\r\n"));
        assert!(manifest
            .uninstall_content
            .unwrap()
            .contains("sc.exe delete"));
    }

    #[test]
    fn cross_platform_relative_path_rejects_windows_drive_and_unc_escape() {
        assert!(validate_cross_platform_relative_path("src/app.rs").is_ok());
        assert!(validate_cross_platform_relative_path("dir with spaces/file.txt").is_ok());
        assert_eq!(
            "WORKSPACE_PATH_ESCAPE",
            validate_cross_platform_relative_path("C:\\Users\\dev\\secret.txt")
                .unwrap_err()
                .code
        );
        assert_eq!(
            "WORKSPACE_PATH_ESCAPE",
            validate_cross_platform_relative_path("\\\\server\\share\\secret.txt")
                .unwrap_err()
                .code
        );
        assert_eq!(
            "WORKSPACE_PATH_OUTSIDE_ROOT",
            validate_cross_platform_relative_path("../secret.txt")
                .unwrap_err()
                .code
        );
    }
}
