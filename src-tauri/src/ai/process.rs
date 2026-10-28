//! Запуск внешнего процесса официального CLI.
//!
//! - только `argv` (без shell), программа — канонический путь из `CliLocator`;
//! - окружение очищается: передаются лишь HOME, USER, LOGNAME, PATH из
//!   фиксированных каталогов, LANG, TMPDIR рабочего каталога, NO_COLOR, TERM;
//!   API-ключи и прочие переменные процесса «Контура» не наследуются;
//! - рабочий каталог — отдельный пустой каталог задания в данных приложения;
//! - промт передаётся через stdin, а не аргументом командной строки;
//! - stdout/stderr читаются в отдельных потоках с ограничением объёма;
//! - тайм-аут и отмена убивают всю группу процессов (SIGKILL).

use std::{
    ffi::OsString,
    io::{Read, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Duration,
    pub stdout_cap: usize,
    pub stderr_cap: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitKind {
    Exited(i32),
    Signaled,
    TimedOut,
    Cancelled,
    SpawnFailed(String),
}

#[derive(Debug, Clone)]
pub struct ProcessOutcome {
    pub exit: ExitKind,
    pub stdout: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr: Vec<u8>,
    pub stderr_truncated: bool,
    pub duration: Duration,
}

/// Периодическое состояние запущенного процесса (без содержимого вывода).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tick {
    pub elapsed: Duration,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
}

/// Безопасное окружение для запуска CLI.
pub fn safe_env(home: Option<&Path>, program: &Path, tmp: &Path) -> Vec<(OsString, OsString)> {
    let mut path_dirs: Vec<String> = Vec::new();
    if let Some(parent) = program.parent() {
        path_dirs.push(parent.to_string_lossy().into_owned());
    }
    for dir in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        if !path_dirs.iter().any(|existing| existing == dir) {
            path_dirs.push(dir.to_owned());
        }
    }
    let mut env = vec![
        (OsString::from("PATH"), OsString::from(path_dirs.join(":"))),
        (OsString::from("LANG"), OsString::from("en_US.UTF-8")),
        (OsString::from("NO_COLOR"), OsString::from("1")),
        (OsString::from("TERM"), OsString::from("dumb")),
        (OsString::from("TMPDIR"), tmp.as_os_str().to_owned()),
    ];
    if let Some(home) = home {
        env.push((OsString::from("HOME"), home.as_os_str().to_owned()));
    }
    // Имя пользователя нужно официальным клиентам для поиска их собственных
    // учётных данных в Связке ключей; секретом оно не является.
    for key in ["USER", "LOGNAME"] {
        if let Some(value) = std::env::var_os(key) {
            env.push((OsString::from(key), value));
        }
    }
    env
}

/// Буфер вывода и признак обрезки по лимиту.
type Captured = Arc<Mutex<(Vec<u8>, bool)>>;

fn spawn_reader(
    mut source: impl Read + Send + 'static,
    cap: usize,
    counter: Arc<AtomicU64>,
    done: mpsc::Sender<()>,
) -> (Captured, thread::JoinHandle<()>) {
    let buffer = Arc::new(Mutex::new((Vec::new(), false)));
    let target = Arc::clone(&buffer);
    let handle = thread::spawn(move || {
        let mut chunk = [0_u8; 8192];
        loop {
            match source.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    counter.fetch_add(read as u64, Ordering::Relaxed);
                    if let Ok(mut guard) = target.lock() {
                        let (data, truncated) = &mut *guard;
                        let room = cap.saturating_sub(data.len());
                        data.extend_from_slice(&chunk[..read.min(room)]);
                        if read > room {
                            *truncated = true;
                        }
                    }
                }
            }
        }
        let _ = done.send(());
    });
    (buffer, handle)
}

fn kill_group(pid: u32) {
    if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
}

/// Запускает процесс и ждёт завершения, тайм-аута или отмены.
pub fn run_process(
    spec: &ProcessSpec,
    cancel: &AtomicBool,
    on_tick: &mut dyn FnMut(Tick),
) -> ProcessOutcome {
    let started = Instant::now();
    let failed = |message: String| ProcessOutcome {
        exit: ExitKind::SpawnFailed(message),
        stdout: Vec::new(),
        stdout_truncated: false,
        stderr: Vec::new(),
        stderr_truncated: false,
        duration: started.elapsed(),
    };
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(spec.env.iter().map(|(key, value)| (key, value)))
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Своя группа: при отмене убиваются и дочерние процессы CLI.
        .process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return failed(format!("процесс не запущен: {}", error.kind())),
    };
    let pid = child.id();

    let (done_tx, done_rx) = mpsc::channel();
    let stdout_bytes = Arc::new(AtomicU64::new(0));
    let stderr_bytes = Arc::new(AtomicU64::new(0));
    let (stdout, stdout_handle) = spawn_reader(
        child.stdout.take().expect("stdout is piped"),
        spec.stdout_cap,
        Arc::clone(&stdout_bytes),
        done_tx.clone(),
    );
    let (stderr, stderr_handle) = spawn_reader(
        child.stderr.take().expect("stderr is piped"),
        spec.stderr_cap,
        Arc::clone(&stderr_bytes),
        done_tx,
    );
    let stdin_handle = match (child.stdin.take(), spec.stdin.clone()) {
        (Some(mut pipe), Some(input)) => Some(thread::spawn(move || {
            // Ошибка записи (процесс закрыл stdin) не фатальна: итог решает код выхода.
            let _ = pipe.write_all(&input);
            drop(pipe);
        })),
        _ => None,
    };

    let mut last_tick = Instant::now();
    let exit = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break match status.code() {
                    Some(code) => ExitKind::Exited(code),
                    None => ExitKind::Signaled,
                }
            }
            Ok(None) => {}
            Err(_) => {
                kill_group(pid);
                let _ = child.kill();
                let _ = child.wait();
                break ExitKind::Signaled;
            }
        }
        let interrupt = if cancel.load(Ordering::SeqCst) {
            Some(ExitKind::Cancelled)
        } else if started.elapsed() >= spec.timeout {
            Some(ExitKind::TimedOut)
        } else {
            None
        };
        if let Some(kind) = interrupt {
            kill_group(pid);
            let _ = child.kill();
            let _ = child.wait();
            break kind;
        }
        if last_tick.elapsed() >= Duration::from_millis(500) {
            last_tick = Instant::now();
            on_tick(Tick {
                elapsed: started.elapsed(),
                stdout_bytes: stdout_bytes.load(Ordering::Relaxed),
                stderr_bytes: stderr_bytes.load(Ordering::Relaxed),
            });
        }
        thread::sleep(Duration::from_millis(50));
    };
    // Добить группу: потомки, оставшиеся после выхода основного процесса, не
    // должны держать каналы вывода.
    kill_group(pid);

    // Потоки чтения завершаются, когда закрываются каналы. Если потомок вышел из
    // группы и держит канал, ждать бесконечно нельзя: 2 с, затем поток остаётся
    // фоновым и завершится сам.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut finished = 0;
    while finished < 2 {
        let left = deadline.saturating_duration_since(Instant::now());
        match done_rx.recv_timeout(left) {
            Ok(()) => finished += 1,
            Err(_) => break,
        }
    }
    if finished == 2 {
        let _ = stdout_handle.join();
        let _ = stderr_handle.join();
    }
    if let Some(handle) = stdin_handle {
        if handle.is_finished() {
            let _ = handle.join();
        }
    }
    let take = |buffer: &Captured| {
        buffer
            .lock()
            .map(|guard| (guard.0.clone(), guard.1))
            .unwrap_or_default()
    };
    let (stdout, stdout_truncated) = take(&stdout);
    let (stderr, stderr_truncated) = take(&stderr);
    ProcessOutcome {
        exit,
        stdout,
        stdout_truncated,
        stderr,
        stderr_truncated,
        duration: started.elapsed(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };

    use tempfile::TempDir;

    use super::{run_process, safe_env, ExitKind, ProcessSpec};

    fn script(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("fake-cli");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn spec(program: PathBuf, cwd: &Path, args: &[&str], stdin: Option<&str>) -> ProcessSpec {
        ProcessSpec {
            env: safe_env(Some(Path::new("/nonexistent-home")), &program, cwd),
            program,
            args: args.iter().map(Into::into).collect(),
            cwd: cwd.to_path_buf(),
            stdin: stdin.map(|text| text.as_bytes().to_vec()),
            timeout: Duration::from_secs(10),
            stdout_cap: 1024,
            stderr_cap: 256,
        }
    }

    #[test]
    fn passes_args_without_shell_and_prompt_through_stdin_in_clean_env() {
        let tmp = TempDir::new().unwrap();
        let work = tmp.path().join("work");
        fs::create_dir(&work).unwrap();
        // Аргумент с «;» и «$(…)» не интерпретируется shell — печатается как есть.
        let program = script(
            tmp.path(),
            r#"printf 'args:%s|%s\n' "$1" "$2"; printf 'cwd:%s\n' "$(pwd)"; cat; printf '\nsecret:%s\n' "${OPENAI_API_KEY:-none}"; printf 'home:%s\n' "$HOME""#,
        );
        std::env::set_var("OPENAI_API_KEY", "sk-test-should-not-leak");
        let outcome = run_process(
            &spec(
                program,
                &work,
                &["a; rm -rf /", "$(id)"],
                Some("ПРОМТ через stdin"),
            ),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        std::env::remove_var("OPENAI_API_KEY");
        let stdout = String::from_utf8(outcome.stdout).unwrap();
        assert_eq!(outcome.exit, ExitKind::Exited(0));
        assert!(stdout.contains("args:a; rm -rf /|$(id)"), "{stdout}");
        assert!(
            stdout.contains(&format!(
                "cwd:{}",
                fs::canonicalize(&work).unwrap().display()
            )) || stdout.contains(&format!("cwd:{}", work.display()))
        );
        assert!(stdout.contains("ПРОМТ через stdin"));
        assert!(stdout.contains("secret:none"), "API key is not inherited");
        assert!(stdout.contains("home:/nonexistent-home"));
    }

    #[test]
    fn caps_output_and_reports_exit_code() {
        let tmp = TempDir::new().unwrap();
        let program = script(tmp.path(), "i=0; while [ $i -lt 400 ]; do echo 0123456789; i=$((i+1)); done; echo boom >&2; exit 3");
        let outcome = run_process(
            &spec(program, tmp.path(), &[], None),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(outcome.exit, ExitKind::Exited(3));
        assert_eq!(outcome.stdout.len(), 1024);
        assert!(outcome.stdout_truncated);
        assert_eq!(String::from_utf8(outcome.stderr).unwrap().trim(), "boom");
    }

    #[test]
    fn timeout_kills_the_whole_process_group() {
        let tmp = TempDir::new().unwrap();
        let marker = tmp.path().join("child-survived");
        let program = script(
            tmp.path(),
            &format!("(sleep 3; touch '{}') & sleep 30", marker.display()),
        );
        let mut request = spec(program, tmp.path(), &[], None);
        request.timeout = Duration::from_millis(300);
        let started = Instant::now();
        let outcome = run_process(&request, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(outcome.exit, ExitKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(6));
        std::thread::sleep(Duration::from_millis(3500));
        assert!(
            !marker.exists(),
            "child of the CLI was killed with the group"
        );
    }

    #[test]
    fn cancel_stops_the_process_and_ticks_report_progress() {
        let tmp = TempDir::new().unwrap();
        let ready = tmp.path().join("ready");
        // Отмена только после того, как скрипт сам сообщил о старте: под нагрузкой
        // фиксированные 600 мс не гарантируют, что `echo` успел выполниться.
        let program = script(
            tmp.path(),
            &format!("echo started; touch '{}'; sleep 30", ready.display()),
        );
        let cancel = AtomicBool::new(false);
        let mut ticks = 0;
        let outcome = run_process(
            &spec(program, tmp.path(), &[], None),
            &cancel,
            &mut |tick| {
                ticks += 1;
                if ready.exists() || tick.elapsed >= Duration::from_secs(20) {
                    cancel.store(true, Ordering::SeqCst);
                }
            },
        );
        assert_eq!(outcome.exit, ExitKind::Cancelled);
        assert!(ticks >= 1);
        assert_eq!(String::from_utf8(outcome.stdout).unwrap(), "started\n");
    }

    #[test]
    fn missing_program_is_a_spawn_failure() {
        let tmp = TempDir::new().unwrap();
        let outcome = run_process(
            &spec(tmp.path().join("absent"), tmp.path(), &[], None),
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert!(matches!(outcome.exit, ExitKind::SpawnFailed(_)));
    }
}
