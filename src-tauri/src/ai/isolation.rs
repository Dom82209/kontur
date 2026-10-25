//! macOS OS-level isolation for every external AI CLI invocation.
//!
//! The runner is deliberately fail-closed: production code can only execute a
//! provider through `/usr/bin/sandbox-exec`, using a deny-by-default Seatbelt
//! profile.  The profile text and job paths are never persisted or exposed in
//! DTOs.  `sandbox-exec` is a deprecated Apple mechanism, so availability is
//! proved at runtime by the self-test below and automatic execution remains an
//! optional, versioned capability.

use std::{
    ffi::OsString,
    fs,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tempfile::TempDir;

use crate::error::{AppError, BackendError};

use super::process::{run_process, ExitKind, ProcessOutcome, ProcessSpec, Tick};

pub const ISOLATION_PROFILE_VERSION: u32 = 1;
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IsolationCheckResult {
    pub id: String,
    pub passed: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IsolationReport {
    pub platform: String,
    pub runner_available: bool,
    pub profile_version: u32,
    pub passed: bool,
    pub checks: Vec<IsolationCheckResult>,
    pub checked_at: String,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IsolationCapability {
    pub available: bool,
    pub profile_version: u32,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderExecutionPolicy {
    ManualOnly,
    IsolatedAutomatic,
}

#[derive(Debug, Clone)]
pub struct IsolationRunner {
    sandbox_exec: PathBuf,
    home: Option<PathBuf>,
}

impl IsolationRunner {
    pub fn system(home: Option<&Path>) -> Self {
        Self {
            sandbox_exec: PathBuf::from(SANDBOX_EXEC),
            home: home.map(Path::to_path_buf),
        }
    }

    #[cfg(test)]
    pub fn with_path(sandbox_exec: PathBuf, home: Option<PathBuf>) -> Self {
        Self { sandbox_exec, home }
    }

    pub fn capability(&self) -> IsolationCapability {
        let reason = self.verify_runner().err();
        IsolationCapability {
            available: reason.is_none(),
            profile_version: ISOLATION_PROFILE_VERSION,
            reason,
        }
    }

    fn verify_runner(&self) -> Result<PathBuf, String> {
        #[cfg(not(target_os = "macos"))]
        return Err("Изолированный автоматический режим доступен только на macOS".to_owned());

        #[cfg(target_os = "macos")]
        {
            let canonical = fs::canonicalize(&self.sandbox_exec)
                .map_err(|_| "Системный isolation runner недоступен".to_owned())?;
            if canonical != Path::new(SANDBOX_EXEC) {
                return Err("Isolation runner имеет неожиданный путь".to_owned());
            }
            let metadata = fs::metadata(&canonical)
                .map_err(|_| "Не удалось проверить isolation runner".to_owned())?;
            let mode = metadata.permissions().mode();
            if !metadata.is_file() || metadata.uid() != 0 || mode & 0o022 != 0 || mode & 0o111 == 0
            {
                return Err("Isolation runner не прошёл проверку владельца и прав".to_owned());
            }
            Ok(canonical)
        }
    }

    fn quote(path: &Path) -> Result<String, AppError> {
        let raw = path
            .to_str()
            .ok_or_else(|| BackendError::blocked("Путь изоляции не является UTF-8"))?;
        if raw.chars().any(char::is_control) {
            return Err(BackendError::blocked(
                "Путь изоляции содержит управляющие символы",
            ));
        }
        Ok(format!(
            "\"{}\"",
            raw.replace('\\', "\\\\").replace('"', "\\\"")
        ))
    }

    fn app_bundle(executable: &Path) -> Option<PathBuf> {
        executable.ancestors().find_map(|ancestor| {
            ancestor
                .extension()
                .is_some_and(|extension| extension == "app")
                .then(|| ancestor.to_path_buf())
        })
    }

    fn credential_paths(&self) -> Vec<PathBuf> {
        let Some(home) = &self.home else {
            return Vec::new();
        };
        let codex_home = home.join(".codex");
        ["auth.json", ".credentials.json"]
            .into_iter()
            .map(|name| codex_home.join(name))
            .filter_map(|path| {
                fs::symlink_metadata(&path)
                    .is_ok_and(|metadata| metadata.file_type().is_file())
                    .then(|| fs::canonicalize(path).ok())
                    .flatten()
            })
            .collect()
    }

    fn profile(&self, executable: &Path, job: &Path) -> Result<String, AppError> {
        let executable = fs::canonicalize(executable)
            .map_err(|_| BackendError::blocked("Исполняемый файл CLI недоступен"))?;
        let job = fs::canonicalize(job)
            .map_err(|_| BackendError::blocked("Каталог AI job недоступен"))?;
        let mut read_rules = vec![
            "(subpath \"/bin\")".to_owned(),
            "(subpath \"/usr/bin\")".to_owned(),
            format!("(subpath {})", Self::quote(&job)?),
            format!("(literal {})", Self::quote(&executable)?),
        ];
        if let Some(bundle) = Self::app_bundle(&executable) {
            read_rules.push(format!("(subpath {})", Self::quote(&bundle)?));
        }

        let mut auth_rules = Vec::new();
        let mut auth_ancestors = Vec::new();
        if self.home.is_some() {
            for path in self.credential_paths() {
                auth_rules.push(format!("(literal {})", Self::quote(&path)?));
                auth_ancestors.push(format!("(path-ancestors {})", Self::quote(&path)?));
            }
            if !auth_rules.is_empty() {
                read_rules.extend(auth_rules);
            }
        }

        let metadata_rule = if auth_ancestors.is_empty() {
            String::new()
        } else {
            format!(
                "(allow file-read-metadata file-test-existence {})\n",
                auth_ancestors.join(" ")
            )
        };
        Ok(format!(
            "(version 1)\n(deny default)\n(import \"system.sb\")\n(system-network)\n\
             (allow network-outbound)\n(allow process*)\n(allow signal (target same-sandbox))\n\
             (allow sysctl-read)\n{}(allow file-read* {})\n(allow file-write* (subpath {}))\n",
            metadata_rule,
            read_rules.join(" "),
            Self::quote(&job)?
        ))
    }

    fn environment(
        &self,
        executable: &Path,
        job: &Path,
    ) -> Result<Vec<(OsString, OsString)>, AppError> {
        let private_home = job.join("home");
        let tmp = job.join("tmp");
        let codex_home = job.join("codex-home");
        for directory in [&private_home, &tmp, &codex_home] {
            fs::create_dir_all(directory).map_err(|error| {
                BackendError::io(format!(
                    "Не удалось подготовить приватное окружение CLI: {}",
                    error.kind()
                ))
            })?;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).map_err(|error| {
                BackendError::io(format!(
                    "Не удалось ограничить приватное окружение CLI: {}",
                    error.kind()
                ))
            })?;
        }
        for credential in self.credential_paths() {
            let Some(name) = credential.file_name() else {
                continue;
            };
            let mounted = codex_home.join(name);
            if fs::read_link(&mounted).is_ok_and(|current| current == credential) {
                continue;
            }
            symlink(&credential, mounted).map_err(|error| {
                BackendError::blocked(format!(
                    "Не удалось предоставить CLI минимальные credentials: {}",
                    error.kind()
                ))
            })?;
        }
        let mut path = executable
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned())
            .into_iter()
            .collect::<Vec<_>>();
        path.extend([
            "/usr/bin".to_owned(),
            "/bin".to_owned(),
            "/usr/sbin".to_owned(),
            "/sbin".to_owned(),
        ]);
        Ok(vec![
            (OsString::from("HOME"), private_home.into_os_string()),
            (OsString::from("TMPDIR"), tmp.into_os_string()),
            (OsString::from("CODEX_HOME"), codex_home.into_os_string()),
            (OsString::from("PATH"), OsString::from(path.join(":"))),
            (OsString::from("LANG"), OsString::from("en_US.UTF-8")),
            (OsString::from("NO_COLOR"), OsString::from("1")),
            (OsString::from("TERM"), OsString::from("dumb")),
        ])
    }

    pub fn run(
        &self,
        target: &ProcessSpec,
        cancel: &AtomicBool,
        on_tick: &mut dyn FnMut(Tick),
    ) -> Result<ProcessOutcome, AppError> {
        let runner = self.verify_runner().map_err(BackendError::blocked)?;
        let profile = self.profile(&target.program, &target.cwd)?;
        let mut args = vec![OsString::from("-p"), OsString::from(profile)];
        args.push(target.program.as_os_str().to_owned());
        args.extend(target.args.iter().cloned());
        Ok(run_process(
            &ProcessSpec {
                program: runner,
                args,
                cwd: target.cwd.clone(),
                env: self.environment(&target.program, &target.cwd)?,
                stdin: target.stdin.clone(),
                timeout: target.timeout,
                stdout_cap: target.stdout_cap,
                stderr_cap: target.stderr_cap,
            },
            cancel,
            on_tick,
        ))
    }
}

pub struct IsolationSelfTest<'a> {
    runner: &'a IsolationRunner,
}

impl<'a> IsolationSelfTest<'a> {
    pub fn new(runner: &'a IsolationRunner) -> Self {
        Self { runner }
    }

    fn check(checks: &mut Vec<IsolationCheckResult>, id: &str, passed: bool, message: &str) {
        checks.push(IsolationCheckResult {
            id: id.to_owned(),
            passed,
            message: message.to_owned(),
        });
    }

    #[cfg(target_os = "macos")]
    pub fn run(&self) -> IsolationReport {
        use std::io::Write;

        let checked_at = super::store::timestamp();
        let capability = self.runner.capability();
        let mut checks = Vec::new();
        Self::check(
            &mut checks,
            "runner",
            capability.available,
            "Системный runner проверен",
        );
        if !capability.available {
            return IsolationReport {
                platform: "macos".to_owned(),
                runner_available: false,
                profile_version: ISOLATION_PROFILE_VERSION,
                passed: false,
                checks,
                checked_at,
                explanation: capability
                    .reason
                    .unwrap_or_else(|| "Isolation runner недоступен".to_owned()),
            };
        }

        let root = match TempDir::new() {
            Ok(root) => root,
            Err(_) => {
                Self::check(
                    &mut checks,
                    "setup",
                    false,
                    "Не удалось создать каталог проверки",
                );
                return IsolationReport {
                    platform: "macos".to_owned(),
                    runner_available: true,
                    profile_version: ISOLATION_PROFILE_VERSION,
                    passed: false,
                    checks,
                    checked_at,
                    explanation: "Проверка изоляции не подготовлена".to_owned(),
                };
            }
        };
        let job = root.path().join("job");
        let other_job = root.path().join("other-job");
        let denied_write = root.path().join("outside-write");
        let sentinel = root.path().join("sentinel");
        let _ = fs::create_dir(&job);
        let _ = fs::create_dir(&other_job);
        let _ = fs::create_dir(&denied_write);
        let _ = fs::set_permissions(&job, fs::Permissions::from_mode(0o700));
        let _ = fs::write(job.join("allowed"), "allowed");
        let _ = fs::write(other_job.join("private"), "other");
        let secret = "KONTUR-ISOLATION-SENTINEL-4f8a";
        let _ = fs::write(&sentinel, secret);
        let fake = job.join("fake-cli");
        let body = r#"#!/bin/sh
case "$1" in
 access)
   cat "$2" >/dev/null && echo READ_ALLOWED
   echo ok > "$3" && echo WRITE_ALLOWED
   cat "$4" >/dev/null 2>&1 && echo SENTINEL_LEAK || echo SENTINEL_BLOCKED
   echo no > "$5" 2>/dev/null && echo WRITE_LEAK || echo WRITE_BLOCKED
   cat "$6" >/dev/null 2>&1 && echo OTHER_LEAK || echo OTHER_BLOCKED
   [ -z "${OPENAI_API_KEY:-}" ] && [ -z "${ANTHROPIC_API_KEY:-}" ] && echo ENV_CLEAN
   for p in "$7" "$8" "$9" "${10}"; do [ -r "$p" ] && echo USER_LEAK; done
   [ -r "${11}" ] && echo CONFIG_LEAK || echo CONFIG_BLOCKED
   ;;
 child)
   (sleep 1; echo survived > "$2") &
   sleep 30
   ;;
esac
"#;
        if fs::File::create(&fake)
            .and_then(|mut file| file.write_all(body.as_bytes()))
            .is_err()
            || fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).is_err()
        {
            Self::check(&mut checks, "setup", false, "Fake CLI не создан");
        } else {
            let home = self
                .runner
                .home
                .clone()
                .unwrap_or_else(|| PathBuf::from("/Users/__kontur_missing__"));
            let args = [
                OsString::from("access"),
                job.join("allowed").into_os_string(),
                job.join("written").into_os_string(),
                sentinel.clone().into_os_string(),
                denied_write.join("blocked").into_os_string(),
                other_job.join("private").into_os_string(),
                home.join("Desktop").into_os_string(),
                home.join("Documents").into_os_string(),
                home.join("Downloads").into_os_string(),
                home.join(".ssh").into_os_string(),
                home.join(".codex/config.toml").into_os_string(),
            ];
            let outcome = self.runner.run(
                &ProcessSpec {
                    program: fake.clone(),
                    args: args.to_vec(),
                    cwd: job.clone(),
                    env: Vec::new(),
                    stdin: None,
                    timeout: Duration::from_secs(5),
                    stdout_cap: 32 * 1024,
                    stderr_cap: 8 * 1024,
                },
                &AtomicBool::new(false),
                &mut |_| {},
            );
            let output = outcome
                .as_ref()
                .map(|item| String::from_utf8_lossy(&item.stdout).into_owned())
                .unwrap_or_default();
            Self::check(
                &mut checks,
                "read_job",
                output.contains("READ_ALLOWED"),
                "Чтение внутри job directory",
            );
            Self::check(
                &mut checks,
                "write_job",
                output.contains("WRITE_ALLOWED") && job.join("written").is_file(),
                "Запись внутри job directory",
            );
            Self::check(
                &mut checks,
                "deny_sentinel",
                output.contains("SENTINEL_BLOCKED") && !output.contains(secret),
                "Sentinel вне job недоступен",
            );
            Self::check(
                &mut checks,
                "deny_write",
                output.contains("WRITE_BLOCKED") && !denied_write.join("blocked").exists(),
                "Запись вне job запрещена",
            );
            Self::check(
                &mut checks,
                "deny_user_dirs",
                !output.contains("USER_LEAK"),
                "Типовые пользовательские каталоги недоступны",
            );
            Self::check(
                &mut checks,
                "deny_user_config",
                output.contains("CONFIG_BLOCKED") && !output.contains("CONFIG_LEAK"),
                "Пользовательский config.toml недоступен",
            );
            Self::check(
                &mut checks,
                "clean_environment",
                output.contains("ENV_CLEAN"),
                "Секретные переменные не наследуются",
            );
            Self::check(
                &mut checks,
                "deny_other_job",
                output.contains("OTHER_BLOCKED"),
                "Другой AI job недоступен",
            );

            let child_marker = job.join("child-survived");
            let mut child_spec = ProcessSpec {
                program: fake,
                args: vec![
                    OsString::from("child"),
                    child_marker.clone().into_os_string(),
                ],
                cwd: job,
                env: Vec::new(),
                stdin: None,
                timeout: Duration::from_millis(250),
                stdout_cap: 1024,
                stderr_cap: 1024,
            };
            let timeout = self
                .runner
                .run(&child_spec, &AtomicBool::new(false), &mut |_| {});
            thread::sleep(Duration::from_millis(1200));
            let timeout_ok = timeout.is_ok_and(|result| result.exit == ExitKind::TimedOut)
                && !child_marker.exists();
            let cancel = AtomicBool::new(true);
            child_spec.timeout = Duration::from_secs(5);
            let cancelled = self.runner.run(&child_spec, &cancel, &mut |_| {});
            thread::sleep(Duration::from_millis(1200));
            let cancel_ok = cancelled.is_ok_and(|result| result.exit == ExitKind::Cancelled)
                && !child_marker.exists();
            Self::check(
                &mut checks,
                "child_process",
                timeout_ok,
                "Дочерний процесс не обходит sandbox",
            );
            Self::check(
                &mut checks,
                "cancel_timeout",
                timeout_ok && cancel_ok,
                "Отмена и timeout завершают process group",
            );
        }

        let passed = checks.len() == 11 && checks.iter().all(|check| check.passed);
        IsolationReport {
            platform: "macos".to_owned(),
            runner_available: true,
            profile_version: ISOLATION_PROFILE_VERSION,
            passed,
            checks,
            checked_at,
            explanation: if passed {
                "Изоляция ограничила файловый доступ тестового CLI приватным job-каталогом. Сеть и переданный текст остаются в зоне провайдера.".to_owned()
            } else {
                "Одна или несколько проверок файловой изоляции не пройдены; автоматический режим заблокирован.".to_owned()
            },
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn run(&self) -> IsolationReport {
        IsolationReport {
            platform: std::env::consts::OS.to_owned(),
            runner_available: false,
            profile_version: ISOLATION_PROFILE_VERSION,
            passed: false,
            checks: vec![IsolationCheckResult {
                id: "platform".to_owned(),
                passed: false,
                message: "Требуется macOS".to_owned(),
            }],
            checked_at: super::store::timestamp(),
            explanation: "Изолированный автоматический режим доступен только на macOS".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{IsolationRunner, IsolationSelfTest, ISOLATION_PROFILE_VERSION};

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_self_test_enforces_job_only_file_access_and_process_group_cleanup() {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let runner = IsolationRunner::system(home.as_deref());
        let report = IsolationSelfTest::new(&runner).run();
        assert_eq!(report.profile_version, ISOLATION_PROFILE_VERSION);
        assert!(report.runner_available, "{report:?}");
        assert!(report.passed, "{report:?}");
        for required in [
            "read_job",
            "write_job",
            "deny_sentinel",
            "deny_write",
            "deny_user_dirs",
            "deny_user_config",
            "clean_environment",
            "deny_other_job",
            "child_process",
            "cancel_timeout",
        ] {
            assert!(
                report
                    .checks
                    .iter()
                    .any(|check| check.id == required && check.passed),
                "missing {required}: {report:?}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn codex_credentials_are_mounted_into_private_home_without_user_config() {
        use std::{ffi::OsString, fs, os::unix::fs::PermissionsExt, sync::atomic::AtomicBool};

        use tempfile::TempDir;

        use crate::ai::process::{ExitKind, ProcessSpec};

        let root = TempDir::new().unwrap();
        let home = root.path().join("user-home");
        let codex = home.join(".codex");
        let job = root.path().join("job");
        fs::create_dir_all(&codex).unwrap();
        fs::create_dir(&job).unwrap();
        fs::write(codex.join("auth.json"), r#"{"token":"test-only"}"#).unwrap();
        fs::write(codex.join("config.toml"), "model = 'must-not-be-read'").unwrap();
        let fake = job.join("codex");
        fs::write(
            &fake,
            r#"#!/bin/sh
[ "$CODEX_HOME" != "$1/.codex" ] || exit 21
[ -r "$CODEX_HOME/auth.json" ] || exit 22
[ ! -r "$1/.codex/config.toml" ] || exit 23
[ ! -e "$CODEX_HOME/config.toml" ] || exit 24
exit 0
"#,
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
        let runner = IsolationRunner::system(Some(&home));
        let outcome = runner
            .run(
                &ProcessSpec {
                    program: fake,
                    args: vec![OsString::from(home.as_os_str())],
                    cwd: job,
                    env: Vec::new(),
                    stdin: None,
                    timeout: std::time::Duration::from_secs(5),
                    stdout_cap: 1024,
                    stderr_cap: 1024,
                },
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(outcome.exit, ExitKind::Exited(0), "{outcome:?}");
    }
}
