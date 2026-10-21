use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

use super::{
    hashing::sha256_file,
    paths::{blocked, io_error},
    SourceFile, VerifiedDestination,
};

const MAX_SUGGESTED_NAME_BYTES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum CopyDecision {
    Copy,
    SameContent,
    NameConflict { suggested_name: String },
}

pub fn plan_copy(
    source: &SourceFile,
    destination: &VerifiedDestination,
) -> Result<CopyDecision, AppError> {
    source.revalidate()?;
    destination.revalidate_parents()?;
    let destination_path = destination.absolute();
    let metadata = match fs::symlink_metadata(&destination_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CopyDecision::Copy)
        }
        Err(_) => return Err(io_error("Не удалось проверить целевой файл")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(blocked(
            "Целевой путь не является безопасным обычным файлом",
        ));
    }
    if sha256_file(source.path())? == sha256_file(&destination_path)? {
        return Ok(CopyDecision::SameContent);
    }
    Ok(CopyDecision::NameConflict {
        suggested_name: next_available_name(destination)?,
    })
}

fn next_available_name(destination: &VerifiedDestination) -> Result<String, AppError> {
    let file_name = destination.relative().file_name().to_string_lossy();
    let file_path = Path::new(file_name.as_ref());
    let stem = file_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Файл");
    let extension = file_path.extension().and_then(|value| value.to_str());

    for index in 2_u32..100_000 {
        let suffix = format!(" ({index})");
        let extension_bytes = extension.map_or(0, |value| value.len() + 1);
        let max_stem = MAX_SUGGESTED_NAME_BYTES.saturating_sub(suffix.len() + extension_bytes);
        let truncated = truncate_utf8(stem, max_stem);
        let candidate = match extension {
            Some(extension) => format!("{truncated}{suffix}.{extension}"),
            None => format!("{truncated}{suffix}"),
        };
        let relative = destination.relative().with_file_name(&candidate)?;
        let candidate_path = destination
            .absolute()
            .parent()
            .expect("validated destination has parent")
            .join(relative.file_name());
        match fs::symlink_metadata(candidate_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => continue,
            Err(_) => return Err(io_error("Не удалось подобрать безопасное имя файла")),
        }
    }
    Err(blocked("Не удалось подобрать свободное имя файла"))
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}
