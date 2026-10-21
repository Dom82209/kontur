use std::{fs::File, io::Read, path::Path};

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;

use super::paths::io_error;

const BUFFER_SIZE: usize = 64 * 1024;

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

pub fn hash_source(source: &super::SourceFile) -> Result<Sha256Digest, AppError> {
    source.revalidate()?;
    sha256_file(source.path())
}

pub(crate) fn sha256_file(path: &Path) -> Result<Sha256Digest, AppError> {
    let mut file =
        File::open(path).map_err(|_| io_error("Не удалось открыть файл для хэширования"))?;
    let metadata = file
        .metadata()
        .map_err(|_| io_error("Не удалось проверить файл для хэширования"))?;
    if !metadata.is_file() {
        return Err(super::paths::blocked(
            "SHA-256 вычисляется только для обычного файла",
        ));
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; BUFFER_SIZE];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| io_error("Ошибка чтения при вычислении SHA-256"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Sha256Digest(format!("{:x}", hasher.finalize())))
}
