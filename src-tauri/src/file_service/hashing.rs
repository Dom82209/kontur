use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn parse(value: impl Into<String>) -> Result<Self, AppError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(super::paths::blocked("Некорректный SHA-256"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(D::Error::custom)
    }
}

impl Sha256Digest {
    pub(crate) fn from_hasher(hasher: Sha256) -> Self {
        Self(format!("{:x}", hasher.finalize()))
    }
}

/// SHA-256 уже открытого source: читается тот же inode, что будет скопирован.
pub fn hash_source(source: &super::SourceFile) -> Result<Sha256Digest, AppError> {
    source.revalidate()?;
    super::fd::hash_open_file(source.file()).map(|(digest, _)| digest)
}

/// Path-based хэш только для тестовых assertions; production-код использует открытые fd.
#[cfg(test)]
pub(crate) fn sha256_file(path: &std::path::Path) -> Result<Sha256Digest, AppError> {
    let file = std::fs::File::open(path)
        .map_err(|_| super::paths::io_error("Не удалось открыть файл для хэширования"))?;
    super::fd::hash_open_file(&file).map(|(digest, _)| digest)
}
