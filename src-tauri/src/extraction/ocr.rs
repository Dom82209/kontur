//! Необязательное локальное распознавание текста на изображениях.
//!
//! OCR выключен по умолчанию и включается только пользователем в Настройках.
//! Используется уже установленный пользователем `tesseract` из фиксированного
//! списка системных каталогов; «Контур» его не скачивает и не устанавливает.
//! Изображение передаётся через stdin, результат читается из stdout — пути
//! файлов процессу не передаются, окружение очищается, время ограничено.
//! Отсутствие OCR не блокирует импорт и извлечение: изображение получает
//! статус `needs_ocr` с метаданными.

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::model::{Budget, Interrupt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrError {
    Interrupted(Interrupt),
    Failed(String),
}

pub trait OcrEngine: Send + Sync {
    fn name(&self) -> &str;
    fn languages(&self) -> &str;
    fn recognize(&self, image: &[u8], budget: &Budget) -> Result<String, OcrError>;
}

/// Каталоги, где установщики пакетов размещают `tesseract` на macOS и Linux.
const TRUSTED_PREFIXES: [&str; 4] = ["/opt/homebrew/", "/usr/local/", "/usr/bin/", "/opt/local/"];
const CANDIDATES: [&str; 4] = [
    "/opt/homebrew/bin/tesseract",
    "/usr/local/bin/tesseract",
    "/opt/local/bin/tesseract",
    "/usr/bin/tesseract",
];
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct TesseractOcr {
    binary: PathBuf,
    languages: String,
}

impl TesseractOcr {
    /// Ищет `tesseract` только в доверенных системных каталогах (не в `PATH`).
    pub fn discover() -> Option<Self> {
        CANDIDATES
            .iter()
            .find_map(|candidate| Self::from_binary(Path::new(candidate)))
    }

    fn from_binary(path: &Path) -> Option<Self> {
        let canonical = std::fs::canonicalize(path).ok()?;
        let text = canonical.to_str()?;
        if !TRUSTED_PREFIXES
            .iter()
            .any(|prefix| text.starts_with(prefix))
        {
            return None;
        }
        let metadata = std::fs::metadata(&canonical).ok()?;
        if !metadata.is_file() {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return None;
            }
        }
        let languages = list_languages(&canonical)?;
        Some(Self {
            binary: canonical,
            languages,
        })
    }
}

fn base_command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("OMP_THREAD_LIMIT", "2")
        .env("LANG", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

/// `rus+eng`, если оба языка установлены; иначе `eng`; без языков — недоступно.
fn list_languages(binary: &Path) -> Option<String> {
    let mut command = base_command(binary);
    command.arg("--list-langs");
    let output = run_limited(command, None, Duration::from_secs(5), None).ok()?;
    let text = String::from_utf8_lossy(&output);
    let installed: Vec<&str> = text
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let wanted: Vec<&str> = ["rus", "eng"]
        .into_iter()
        .filter(|language| installed.contains(language))
        .collect();
    (!wanted.is_empty()).then(|| wanted.join("+"))
}

impl OcrEngine for TesseractOcr {
    fn name(&self) -> &str {
        "tesseract"
    }

    fn languages(&self) -> &str {
        &self.languages
    }

    fn recognize(&self, image: &[u8], budget: &Budget) -> Result<String, OcrError> {
        let mut command = base_command(&self.binary);
        command
            .args(["stdin", "stdout", "-l", &self.languages, "--psm", "3"])
            .stdin(Stdio::piped());
        let remaining = budget
            .deadline()
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(120));
        let output = run_limited(command, Some(image.to_vec()), remaining, Some(budget))?;
        Ok(String::from_utf8_lossy(&output).into_owned())
    }
}

/// Запускает процесс с ограничением времени и размера вывода; при отмене
/// или превышении времени процесс убивается.
fn run_limited(
    mut command: Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
    budget: Option<&Budget>,
) -> Result<Vec<u8>, OcrError> {
    let mut child = command
        .spawn()
        .map_err(|_| OcrError::Failed("Не удалось запустить OCR".into()))?;
    let writer = input.and_then(|data| {
        child.stdin.take().map(|mut stdin| {
            thread::spawn(move || {
                let _ = stdin.write_all(&data);
            })
        })
    });
    let reader = child.stdout.take().map(|stdout| {
        thread::spawn(move || {
            let mut output = Vec::new();
            let _ = stdout
                .take(MAX_OUTPUT_BYTES as u64)
                .read_to_end(&mut output);
            output
        })
    });
    let started = Instant::now();
    let status = loop {
        if let Some(budget) = budget {
            if let Err(interrupt) = budget.check() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(OcrError::Interrupted(interrupt));
            }
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(OcrError::Interrupted(Interrupt::Timeout));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(_) => return Err(OcrError::Failed("OCR завершился с ошибкой".into())),
        }
    };
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let output = reader
        .map(|reader| reader.join().unwrap_or_default())
        .unwrap_or_default();
    if !status.success() {
        return Err(OcrError::Failed(
            "OCR не смог распознать изображение".into(),
        ));
    }
    Ok(output)
}

/// Состояние OCR для Настроек (без путей к исполняемому файлу).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrStatusDto {
    pub available: bool,
    pub engine: Option<String>,
    pub languages: Option<String>,
    pub message: String,
}

pub fn status_of(engine: Option<&dyn OcrEngine>) -> OcrStatusDto {
    match engine {
        Some(engine) => OcrStatusDto {
            available: true,
            engine: Some(engine.name().to_owned()),
            languages: Some(engine.languages().to_owned()),
            message: format!(
                "Найден локальный {} ({}). Изображения распознаются на этом компьютере",
                engine.name(),
                engine.languages()
            ),
        },
        None => OcrStatusDto {
            available: false,
            engine: None,
            languages: None,
            message: "Локальный OCR не найден. Можно установить Tesseract (например, `brew install tesseract tesseract-lang`); без него изображения сохраняются с метаданными".into(),
        },
    }
}
