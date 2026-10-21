use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

use super::{
    fd::{self, DirFd, Leaf},
    hooks::{self, HookPoint},
    paths::blocked,
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

/// Read-only planning: открывает родителя назначения fd-relative, сравнивает
/// SHA-256 открытого source и открытого destination, подбирает свободное имя
/// через `statat` в том же каталоге и в конце подтверждает, что каталог не подменён.
pub fn plan_copy(
    source: &SourceFile,
    destination: &VerifiedDestination,
) -> Result<CopyDecision, AppError> {
    source.revalidate()?;
    let Some(parent) = destination.open_existing_parent()? else {
        return Ok(CopyDecision::Copy);
    };
    hooks::fire(|| HookPoint::PlanningParentOpened);
    let decision = match fd::open_leaf(&parent, destination.file_name())? {
        Leaf::Missing => CopyDecision::Copy,
        Leaf::NotRegular => {
            return Err(blocked(
                "Целевой путь не является безопасным обычным файлом",
            ))
        }
        Leaf::File(existing, _) => {
            let (source_hash, _) = fd::hash_open_file(source.file())?;
            let (existing_hash, _) = fd::hash_open_file(&existing)?;
            if source_hash == existing_hash {
                CopyDecision::SameContent
            } else {
                CopyDecision::NameConflict {
                    suggested_name: next_available_name(&parent, destination)?,
                }
            }
        }
    };
    destination.verify_parent(&parent)?;
    Ok(decision)
}

fn next_available_name(
    parent: &DirFd,
    destination: &VerifiedDestination,
) -> Result<String, AppError> {
    let file_name = destination.file_name().to_string_lossy();
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
        if !fd::entry_exists(parent, relative.file_name())? {
            return Ok(candidate);
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
