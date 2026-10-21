//! Безопасный разбор ZIP-архивов экспорта LMS.
//!
//! Собственный строгий парсер центрального каталога вместо универсальной
//! библиотеки: архив из загрузок недоверенный, поэтому любая неоднозначность
//! отклоняет его целиком, а не «исправляется».
//!
//! Проверки индекса (без распаковки):
//!
//! - ровно одна запись EOCD в конце файла, без многотомности и ZIP64;
//! - лимиты размера архива, числа записей, размера записи, суммарного размера
//!   и коэффициента сжатия (по заголовкам);
//! - имя: не пустое, UTF-8 (или CP866 для старых архиваторов), без NUL и
//!   управляющих символов, без `\`, абсолютного пути, drive prefix, `.`/`..`
//!   и пустых компонентов, с ограничением длины и глубины;
//! - symlink, устройства, FIFO и сокеты по Unix-атрибутам → архив отклоняется;
//! - зашифрованные записи и методы кроме stored/deflate → отклоняется;
//! - совпадающие пути (без учёта регистра) и конфликт «файл/каталог» → отклоняется;
//! - локальный заголовок совпадает с центральным, данные записей не перекрываются
//!   и не заходят в центральный каталог (защита от overlapping-bomb).
//!
//! Распаковка ([`extract_entry`]) не доверяет заголовкам: поток ограничен
//! заявленным размером и лимитами, CRC-32 и размер сверяются после записи.
//! Вложенные архивы никогда не распаковываются.

use std::{collections::HashMap, fs::File, io::Read, os::unix::fs::FileExt};

use sha2::{Digest, Sha256};

use crate::{error::AppError, file_service::Sha256Digest};

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
const EOCD_MIN: u64 = 22;
const MAX_COMMENT: u64 = 0xFFFF;
const CENTRAL_FIXED: usize = 46;
const LOCAL_FIXED: usize = 30;

const FLAG_ENCRYPTED: u16 = 0x0001;
const FLAG_STRONG_ENCRYPTION: u16 = 0x0040;
const FLAG_UTF8: u16 = 0x0800;
const FLAG_MASKED_HEADER: u16 = 0x2000;

const S_IFMT: u32 = 0o170_000;
const S_IFLNK: u32 = 0o120_000;
const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;

/// Лимиты разбора и распаковки. Значения по умолчанию рассчитаны на учебный курс.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLimits {
    pub max_archive_bytes: u64,
    pub max_entries: usize,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    /// Предельное отношение распакованного размера к сжатому.
    pub max_ratio: u64,
    /// Коэффициент сжатия проверяется для записей не меньше этого размера:
    /// маленькие текстовые файлы законно сжимаются сильнее.
    pub ratio_floor_bytes: u64,
    pub max_path_bytes: usize,
    pub max_depth: usize,
}

impl Default for ZipLimits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 2 * 1024 * 1024 * 1024,
            max_entries: 5_000,
            max_entry_bytes: 1024 * 1024 * 1024,
            max_total_bytes: 4 * 1024 * 1024 * 1024,
            max_ratio: 200,
            ratio_floor_bytes: 1024 * 1024,
            max_path_bytes: 1024,
            max_depth: 16,
        }
    }
}

/// Причина отказа. Сообщение пользователю — [`ZipRejection::message`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipRejection {
    NotZip,
    ArchiveTooLarge,
    MultiDisk,
    Zip64,
    TooManyEntries,
    Corrupt(&'static str),
    Encrypted,
    UnsupportedMethod(u16),
    Symlink(String),
    SpecialFile(String),
    UnsafePath(UnsafePath),
    DuplicatePath(String),
    EntryTooLarge(String),
    TotalTooLarge,
    RatioTooHigh(String),
    Overlap,
    NestedArchive(String),
    SizeMismatch(String),
    CrcMismatch(String),
    Io,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsafePath {
    Empty,
    Encoding,
    Control,
    Backslash,
    Absolute,
    DrivePrefix,
    Traversal,
    EmptyComponent,
    TooLong,
    TooDeep,
}

impl ZipRejection {
    pub fn message(&self) -> String {
        match self {
            Self::NotZip => "Файл не является ZIP-архивом".to_owned(),
            Self::ArchiveTooLarge => "Архив больше допустимого размера".to_owned(),
            Self::MultiDisk => "Многотомные архивы не поддерживаются".to_owned(),
            Self::Zip64 => {
                "Архивы ZIP64 (больше 4 ГБ или 65 535 записей) не поддерживаются".to_owned()
            }
            Self::TooManyEntries => "В архиве слишком много записей".to_owned(),
            Self::Corrupt(reason) => format!("Архив повреждён: {reason}"),
            Self::Encrypted => "Зашифрованные архивы не поддерживаются".to_owned(),
            Self::UnsupportedMethod(method) => {
                format!("Метод сжатия {method} не поддерживается (разрешены stored и deflate)")
            }
            Self::Symlink(name) => {
                format!("Архив содержит символическую ссылку «{name}» — отклонён")
            }
            Self::SpecialFile(name) => {
                format!("Архив содержит специальный файл «{name}» — отклонён")
            }
            Self::UnsafePath(reason) => format!("Небезопасное имя в архиве: {}", reason.message()),
            Self::DuplicatePath(name) => {
                format!("Путь «{name}» встречается в архиве дважды — архив отклонён")
            }
            Self::EntryTooLarge(name) => format!("Файл «{name}» в архиве слишком большой"),
            Self::TotalTooLarge => {
                "Суммарный распакованный размер архива слишком большой".to_owned()
            }
            Self::RatioTooHigh(name) => {
                format!("Подозрительно высокое сжатие у «{name}» (возможна zip-бомба)")
            }
            Self::Overlap => "Данные записей архива перекрываются (возможна zip-бомба)".to_owned(),
            Self::NestedArchive(name) => {
                format!("«{name}» — вложенный архив; вложенные архивы не распаковываются")
            }
            Self::SizeMismatch(name) => {
                format!("Размер «{name}» не совпадает с заголовком архива")
            }
            Self::CrcMismatch(name) => format!("Контрольная сумма «{name}» не совпадает"),
            Self::Io => "Не удалось прочитать архив".to_owned(),
        }
    }

    pub fn into_app_error(self) -> AppError {
        AppError::blocked(self.message())
    }
}

impl UnsafePath {
    fn message(self) -> &'static str {
        match self {
            Self::Empty => "пустое имя",
            Self::Encoding => "имя в неизвестной кодировке",
            Self::Control => "управляющие символы",
            Self::Backslash => "обратная косая черта",
            Self::Absolute => "абсолютный путь",
            Self::DrivePrefix => "буква диска",
            Self::Traversal => "компонент «.» или «..»",
            Self::EmptyComponent => "пустой компонент пути",
            Self::TooLong => "слишком длинный путь",
            Self::TooDeep => "слишком глубокая вложенность",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Stored,
    Deflate,
}

/// Запись архива после всех проверок индекса.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    pub index: usize,
    /// Нормализованный относительный путь внутри архива (`/`-разделители, без `/` в конце).
    pub path: String,
    pub is_dir: bool,
    pub method: Method,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub crc32: u32,
    data_offset: u64,
}

impl ZipEntry {
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// Индекс архива: проверенные записи и то, что пропущено как служебное.
#[derive(Debug, Clone)]
pub struct ZipIndex {
    pub entries: Vec<ZipEntry>,
    /// Служебные файлы архиваторов (`__MACOSX/`, `.DS_Store`, `._*`).
    pub ignored: Vec<String>,
    pub total_uncompressed: u64,
}

impl ZipIndex {
    pub fn files(&self) -> impl Iterator<Item = &ZipEntry> {
        self.entries.iter().filter(|entry| !entry.is_dir)
    }
}

struct RawCentral {
    flags: u16,
    method: u16,
    crc32: u32,
    compressed: u64,
    uncompressed: u64,
    name_raw: Vec<u8>,
    external: u32,
    host: u8,
    local_offset: u64,
}

fn read_exact_at(file: &File, offset: u64, length: usize) -> Result<Vec<u8>, ZipRejection> {
    let mut buffer = vec![0_u8; length];
    file.read_exact_at(&mut buffer, offset)
        .map_err(|_| ZipRejection::Corrupt("неожиданный конец файла"))?;
    Ok(buffer)
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Читает и проверяет индекс архива. Содержимое записей не распаковывается.
pub fn read_index(file: &File, limits: &ZipLimits) -> Result<ZipIndex, ZipRejection> {
    let length = file.metadata().map_err(|_| ZipRejection::Io)?.len();
    if length > limits.max_archive_bytes {
        return Err(ZipRejection::ArchiveTooLarge);
    }
    if length < EOCD_MIN {
        return Err(ZipRejection::NotZip);
    }
    let tail_length = length.min(EOCD_MIN + MAX_COMMENT);
    let tail_start = length - tail_length;
    let tail = read_exact_at(file, tail_start, tail_length as usize)?;

    // Последняя сигнатура EOCD, чей комментарий заканчивается ровно на конце файла.
    let eocd_relative = (0..=tail.len() - EOCD_MIN as usize)
        .rev()
        .find(|&position| {
            u32_at(&tail, position) == EOCD_SIGNATURE
                && position + EOCD_MIN as usize + u16_at(&tail, position + 20) as usize
                    == tail.len()
        })
        .ok_or(ZipRejection::NotZip)?;
    let eocd = &tail[eocd_relative..eocd_relative + EOCD_MIN as usize];
    let eocd_position = tail_start + eocd_relative as u64;

    let disk = u16_at(eocd, 4);
    let cd_disk = u16_at(eocd, 6);
    let entries_on_disk = u16_at(eocd, 8);
    let entries_total = u16_at(eocd, 10);
    let cd_size = u64::from(u32_at(eocd, 12));
    let cd_offset = u64::from(u32_at(eocd, 16));
    if entries_total == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
        return Err(ZipRejection::Zip64);
    }
    if eocd_position >= 20 {
        let locator = read_exact_at(file, eocd_position - 20, 4)?;
        if u32_at(&locator, 0) == ZIP64_LOCATOR_SIGNATURE {
            return Err(ZipRejection::Zip64);
        }
    }
    if disk != 0 || cd_disk != 0 || entries_on_disk != entries_total {
        return Err(ZipRejection::MultiDisk);
    }
    if usize::from(entries_total) > limits.max_entries {
        return Err(ZipRejection::TooManyEntries);
    }
    if cd_offset
        .checked_add(cd_size)
        .is_none_or(|end| end != eocd_position)
    {
        return Err(ZipRejection::Corrupt(
            "центральный каталог не там, где указано",
        ));
    }

    let directory = read_exact_at(file, cd_offset, cd_size as usize)?;
    let mut raws = Vec::with_capacity(usize::from(entries_total));
    let mut cursor = 0_usize;
    for _ in 0..entries_total {
        if cursor + CENTRAL_FIXED > directory.len() {
            return Err(ZipRejection::Corrupt("обрезанный центральный каталог"));
        }
        let header = &directory[cursor..cursor + CENTRAL_FIXED];
        if u32_at(header, 0) != CENTRAL_SIGNATURE {
            return Err(ZipRejection::Corrupt("неверная сигнатура записи каталога"));
        }
        let made_by = u16_at(header, 4);
        let flags = u16_at(header, 8);
        let method = u16_at(header, 10);
        let crc32 = u32_at(header, 16);
        let compressed = u64::from(u32_at(header, 20));
        let uncompressed = u64::from(u32_at(header, 24));
        let name_length = usize::from(u16_at(header, 28));
        let extra_length = usize::from(u16_at(header, 30));
        let comment_length = usize::from(u16_at(header, 32));
        let start_disk = u16_at(header, 34);
        let external = u32_at(header, 38);
        let local_offset = u64::from(u32_at(header, 42));
        if compressed == 0xFFFF_FFFF || uncompressed == 0xFFFF_FFFF || local_offset == 0xFFFF_FFFF {
            return Err(ZipRejection::Zip64);
        }
        if start_disk != 0 {
            return Err(ZipRejection::MultiDisk);
        }
        let variable = name_length + extra_length + comment_length;
        if cursor + CENTRAL_FIXED + variable > directory.len() {
            return Err(ZipRejection::Corrupt("обрезанный центральный каталог"));
        }
        let name_raw =
            directory[cursor + CENTRAL_FIXED..cursor + CENTRAL_FIXED + name_length].to_vec();
        raws.push(RawCentral {
            flags,
            method,
            crc32,
            compressed,
            uncompressed,
            name_raw,
            external,
            host: (made_by >> 8) as u8,
            local_offset,
        });
        cursor += CENTRAL_FIXED + variable;
    }
    if cursor != directory.len() {
        return Err(ZipRejection::Corrupt(
            "размер центрального каталога не совпадает",
        ));
    }

    let mut entries = Vec::with_capacity(raws.len());
    let mut ignored = Vec::new();
    let mut seen: HashMap<String, bool> = HashMap::new();
    let mut total_uncompressed = 0_u64;
    let mut total_compressed = 0_u64;
    for (index, raw) in raws.into_iter().enumerate() {
        if raw.flags & (FLAG_ENCRYPTED | FLAG_STRONG_ENCRYPTION | FLAG_MASKED_HEADER) != 0 {
            return Err(ZipRejection::Encrypted);
        }
        let method = match raw.method {
            0 => Method::Stored,
            8 => Method::Deflate,
            other => return Err(ZipRejection::UnsupportedMethod(other)),
        };
        let decoded = decode_name(&raw.name_raw, raw.flags & FLAG_UTF8 != 0)
            .ok_or(ZipRejection::UnsafePath(UnsafePath::Encoding))?;
        let mut is_dir = decoded.ends_with('/');
        let unix_mode = if matches!(raw.host, 3 | 19) {
            raw.external >> 16
        } else {
            0
        };
        match unix_mode & S_IFMT {
            0 | S_IFREG => {}
            S_IFDIR => is_dir = true,
            S_IFLNK => return Err(ZipRejection::Symlink(lossy_display(&decoded))),
            _ => return Err(ZipRejection::SpecialFile(lossy_display(&decoded))),
        }
        if raw.host == 0 && raw.external & 0x10 != 0 {
            is_dir = true;
        }
        let path = sanitize_entry_path(&decoded, limits).map_err(ZipRejection::UnsafePath)?;
        if is_dir && (raw.uncompressed != 0 || raw.compressed > 2) {
            return Err(ZipRejection::Corrupt("каталог с данными"));
        }
        if method == Method::Stored && raw.compressed != raw.uncompressed {
            return Err(ZipRejection::Corrupt("размеры несжатой записи различаются"));
        }

        // Совпадения путей и конфликты «файл/каталог» без учёта регистра (APFS).
        let key = path.to_lowercase();
        if seen.get(&key).is_some_and(|dir| !(*dir && is_dir)) {
            return Err(ZipRejection::DuplicatePath(path));
        }
        let mut prefix = String::new();
        for component in key
            .split('/')
            .take(key.split('/').count().saturating_sub(1))
        {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            if seen.get(&prefix) == Some(&false) {
                return Err(ZipRejection::DuplicatePath(path));
            }
            seen.entry(prefix.clone()).or_insert(true);
        }
        if !is_dir && seen.get(&key) == Some(&true) {
            return Err(ZipRejection::DuplicatePath(path));
        }
        seen.insert(key, is_dir);

        if !is_dir {
            if raw.uncompressed > limits.max_entry_bytes {
                return Err(ZipRejection::EntryTooLarge(path));
            }
            if raw.uncompressed >= limits.ratio_floor_bytes
                && raw.uncompressed / raw.compressed.max(1) > limits.max_ratio
            {
                return Err(ZipRejection::RatioTooHigh(path));
            }
            total_uncompressed = total_uncompressed.saturating_add(raw.uncompressed);
            total_compressed = total_compressed.saturating_add(raw.compressed);
            if total_uncompressed > limits.max_total_bytes {
                return Err(ZipRejection::TotalTooLarge);
            }
        }

        // Локальный заголовок обязан совпадать с центральным.
        let local = read_exact_at(file, raw.local_offset, LOCAL_FIXED)?;
        if u32_at(&local, 0) != LOCAL_SIGNATURE {
            return Err(ZipRejection::Corrupt(
                "неверная сигнатура локального заголовка",
            ));
        }
        let local_flags = u16_at(&local, 6);
        if local_flags & (FLAG_ENCRYPTED | FLAG_STRONG_ENCRYPTION | FLAG_MASKED_HEADER) != 0 {
            return Err(ZipRejection::Encrypted);
        }
        if u16_at(&local, 8) != raw.method {
            return Err(ZipRejection::Corrupt(
                "метод сжатия в заголовках различается",
            ));
        }
        let local_name_length = u64::from(u16_at(&local, 26));
        let local_extra_length = u64::from(u16_at(&local, 28));
        let local_name = read_exact_at(
            file,
            raw.local_offset + LOCAL_FIXED as u64,
            local_name_length as usize,
        )?;
        if local_name != raw.name_raw {
            return Err(ZipRejection::Corrupt("имена в заголовках различаются"));
        }
        let data_offset =
            raw.local_offset + LOCAL_FIXED as u64 + local_name_length + local_extra_length;
        let data_end = data_offset
            .checked_add(raw.compressed)
            .ok_or(ZipRejection::Corrupt("переполнение смещения"))?;
        if data_end > cd_offset {
            return Err(ZipRejection::Overlap);
        }

        if is_ignored_path(&path) {
            ignored.push(path);
            // Служебная запись всё равно учитывается в проверке перекрытий ниже.
            entries.push(ZipEntry {
                index,
                path: String::new(),
                is_dir: true,
                method,
                compressed_size: raw.compressed,
                uncompressed_size: 0,
                crc32: raw.crc32,
                data_offset,
            });
            continue;
        }
        entries.push(ZipEntry {
            index,
            path,
            is_dir,
            method,
            compressed_size: raw.compressed,
            uncompressed_size: raw.uncompressed,
            crc32: raw.crc32,
            data_offset,
        });
    }
    if total_uncompressed >= limits.ratio_floor_bytes
        && total_uncompressed / total_compressed.max(1) > limits.max_ratio
    {
        return Err(ZipRejection::RatioTooHigh("архив целиком".to_owned()));
    }

    // Данные записей не должны перекрываться: один и тот же поток нельзя
    // переиспользовать для нескольких «файлов» (overlapping zip bomb).
    let mut spans: Vec<(u64, u64)> = entries
        .iter()
        .filter(|entry| entry.compressed_size > 0)
        .map(|entry| (entry.data_offset, entry.data_offset + entry.compressed_size))
        .collect();
    spans.sort_unstable();
    if spans.windows(2).any(|pair| pair[1].0 < pair[0].1) {
        return Err(ZipRejection::Overlap);
    }

    entries.retain(|entry| !entry.path.is_empty());
    Ok(ZipIndex {
        entries,
        ignored,
        total_uncompressed,
    })
}

/// Распаковывает одну запись в `output` и возвращает (размер, SHA-256).
///
/// Поток ограничен заявленным размером и лимитом записи: архив, который при
/// распаковке даёт больше байт, чем указано в заголовке, отклоняется.
/// `budget` — оставшийся суммарный лимит распакованных байт для всей операции.
pub fn extract_entry(
    archive: &File,
    entry: &ZipEntry,
    output: &mut File,
    limits: &ZipLimits,
    budget: &mut u64,
) -> Result<(u64, Sha256Digest), ZipRejection> {
    use std::io::Write;

    if entry.is_dir {
        return Err(ZipRejection::Corrupt("каталог нельзя распаковать как файл"));
    }
    let raw = PreadSlice {
        file: archive,
        position: entry.data_offset,
        end: entry.data_offset + entry.compressed_size,
    };
    let mut reader: Box<dyn Read + '_> = match entry.method {
        Method::Stored => Box::new(raw),
        Method::Deflate => Box::new(flate2::read::DeflateDecoder::new(raw)),
    };
    let ceiling = entry
        .uncompressed_size
        .min(limits.max_entry_bytes)
        .min(*budget);
    let mut crc = crc32fast::Hasher::new();
    let mut sha = Sha256::new();
    let mut written = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    let check_magic = !is_document_container(&entry.path);
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|_| ZipRejection::Corrupt("ошибка распаковки"))?;
        if read == 0 {
            break;
        }
        if written == 0 && check_magic && looks_like_archive(&buffer[..read]) {
            return Err(ZipRejection::NestedArchive(entry.path.clone()));
        }
        written += read as u64;
        if written > ceiling {
            return Err(if written > entry.uncompressed_size {
                ZipRejection::SizeMismatch(entry.path.clone())
            } else if written > limits.max_entry_bytes {
                ZipRejection::EntryTooLarge(entry.path.clone())
            } else {
                ZipRejection::TotalTooLarge
            });
        }
        crc.update(&buffer[..read]);
        sha.update(&buffer[..read]);
        output
            .write_all(&buffer[..read])
            .map_err(|_| ZipRejection::Io)?;
    }
    if written != entry.uncompressed_size {
        return Err(ZipRejection::SizeMismatch(entry.path.clone()));
    }
    if crc.finalize() != entry.crc32 {
        return Err(ZipRejection::CrcMismatch(entry.path.clone()));
    }
    output.sync_all().map_err(|_| ZipRejection::Io)?;
    *budget -= written;
    Ok((written, Sha256Digest::from_hasher(sha)))
}

/// Читает заданный диапазон файла позиционными чтениями (`pread`).
struct PreadSlice<'a> {
    file: &'a File,
    position: u64,
    end: u64,
}

impl Read for PreadSlice<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let remaining = self.end.saturating_sub(self.position);
        if remaining == 0 {
            return Ok(0);
        }
        let length = buffer.len().min(remaining as usize);
        let read = self.file.read_at(&mut buffer[..length], self.position)?;
        if read == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        self.position += read as u64;
        Ok(read)
    }
}

/// Проверяет и нормализует путь записи архива.
pub fn sanitize_entry_path(name: &str, limits: &ZipLimits) -> Result<String, UnsafePath> {
    if name.is_empty() {
        return Err(UnsafePath::Empty);
    }
    if name
        .chars()
        .any(|ch| ch.is_control() || is_bidi_control(ch))
    {
        return Err(UnsafePath::Control);
    }
    if name.contains('\\') {
        return Err(UnsafePath::Backslash);
    }
    if name.starts_with('/') {
        return Err(UnsafePath::Absolute);
    }
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(UnsafePath::DrivePrefix);
    }
    if name.len() > limits.max_path_bytes {
        return Err(UnsafePath::TooLong);
    }
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() {
        return Err(UnsafePath::Empty);
    }
    let components: Vec<&str> = trimmed.split('/').collect();
    if components.len() > limits.max_depth {
        return Err(UnsafePath::TooDeep);
    }
    for component in &components {
        if component.is_empty() {
            return Err(UnsafePath::EmptyComponent);
        }
        if *component == "." || *component == ".." {
            return Err(UnsafePath::Traversal);
        }
        if component.len() > 255 {
            return Err(UnsafePath::TooLong);
        }
    }
    Ok(components.join("/"))
}

fn is_bidi_control(ch: char) -> bool {
    matches!(ch, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}')
}

fn lossy_display(name: &str) -> String {
    name.chars()
        .filter(|ch| !ch.is_control())
        .take(120)
        .collect()
}

/// Служебные файлы архиваторов, которые не являются материалами курса.
pub fn is_ignored_path(path: &str) -> bool {
    let mut components = path.split('/');
    let file = path.rsplit('/').next().unwrap_or(path);
    components.any(|component| component == "__MACOSX")
        || file == ".DS_Store"
        || file == "Thumbs.db"
        || file == "desktop.ini"
        || file.starts_with("._")
}

/// Расширения архивов, которые никогда не распаковываются рекурсивно.
pub fn is_archive_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        ".zip", ".rar", ".7z", ".tar", ".gz", ".tgz", ".bz2", ".tbz", ".xz", ".txz", ".zst", ".lz",
        ".lzma", ".cab", ".iso", ".dmg", ".jar", ".war", ".apk", ".xpi", ".crx",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

/// Документы, которые по формату являются ZIP-контейнерами (их не распаковываем,
/// но и не считаем вложенным архивом).
fn is_document_container(name: &str) -> bool {
    let lower = name.to_lowercase();
    [
        ".docx", ".xlsx", ".pptx", ".docm", ".xlsm", ".pptm", ".odt", ".ods", ".odp", ".epub",
        ".key", ".numbers", ".pages", ".ipynb",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

/// Сигнатуры архивов в начале потока (ZIP, RAR, 7z, gzip, bzip2, xz, zstd).
fn looks_like_archive(head: &[u8]) -> bool {
    head.starts_with(b"PK\x03\x04")
        || head.starts_with(b"PK\x05\x06")
        || head.starts_with(b"Rar!")
        || head.starts_with(b"7z\xBC\xAF\x27\x1C")
        || head.starts_with(&[0x1F, 0x8B])
        || head.starts_with(b"BZh")
        || head.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00])
        || head.starts_with(&[0x28, 0xB5, 0x2F, 0xFD])
}

/// Имя записи: UTF-8 (флаг 11 или валидные байты), иначе CP866 — так архивируют
/// русские имена старые Windows-архиваторы. Нераспознанные байты → `None`.
fn decode_name(raw: &[u8], utf8_flag: bool) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(raw) {
        return Some(text.to_owned());
    }
    if utf8_flag {
        return None;
    }
    raw.iter().map(|&byte| decode_cp866(byte)).collect()
}

fn decode_cp866(byte: u8) -> Option<char> {
    match byte {
        0x00..=0x7F => Some(byte as char),
        // А–Я, а–п
        0x80..=0xAF => char::from_u32(0x0410 + u32::from(byte - 0x80)),
        // р–я
        0xE0..=0xEF => char::from_u32(0x0440 + u32::from(byte - 0xE0)),
        0xF0 => Some('Ё'),
        0xF1 => Some('ё'),
        0xFC => Some('№'),
        0xFF => Some('\u{00A0}'),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod test_zip {
    //! Конструктор «сырых» ZIP-архивов для тестов, включая заведомо вредоносные.

    use std::io::Write;

    #[derive(Clone)]
    pub struct RawEntry {
        pub name: Vec<u8>,
        pub local_name: Option<Vec<u8>>,
        pub data: Vec<u8>,
        pub deflate: bool,
        pub flags: u16,
        pub made_by: u16,
        pub external: u32,
        pub declared_uncompressed: Option<u32>,
        pub declared_crc: Option<u32>,
        pub method: Option<u16>,
    }

    impl RawEntry {
        pub fn file(name: &str, data: &[u8]) -> Self {
            Self {
                name: name.as_bytes().to_vec(),
                local_name: None,
                data: data.to_vec(),
                deflate: true,
                flags: 0x0800,
                made_by: 0x031E,
                external: 0o100_644 << 16,
                declared_uncompressed: None,
                declared_crc: None,
                method: None,
            }
        }

        pub fn dir(name: &str) -> Self {
            Self {
                external: (0o040_755 << 16) | 0x10,
                deflate: false,
                ..Self::file(name, b"")
            }
        }

        pub fn symlink(name: &str, target: &str) -> Self {
            Self {
                external: 0o120_777 << 16,
                deflate: false,
                ..Self::file(name, target.as_bytes())
            }
        }
    }

    pub fn deflate(data: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    /// Собирает архив. `share_data` — все записи ссылаются на данные первой
    /// (overlapping bomb).
    pub fn build(entries: &[RawEntry], share_data: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        let mut first_offset = None;
        for entry in entries {
            let payload = if entry.deflate {
                deflate(&entry.data)
            } else {
                entry.data.clone()
            };
            let method = entry.method.unwrap_or(if entry.deflate { 8 } else { 0 });
            let crc = entry
                .declared_crc
                .unwrap_or_else(|| crc32fast::hash(&entry.data));
            let uncompressed = entry
                .declared_uncompressed
                .unwrap_or(entry.data.len() as u32);
            let local_name = entry.local_name.clone().unwrap_or(entry.name.clone());
            let offset = if share_data {
                if let Some(offset) = first_offset {
                    offset
                } else {
                    let offset = out.len() as u32;
                    write_local(
                        &mut out,
                        entry.flags,
                        method,
                        crc,
                        &payload,
                        uncompressed,
                        &local_name,
                    );
                    first_offset = Some(offset);
                    offset
                }
            } else {
                let offset = out.len() as u32;
                write_local(
                    &mut out,
                    entry.flags,
                    method,
                    crc,
                    &payload,
                    uncompressed,
                    &local_name,
                );
                offset
            };
            central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
            central.extend_from_slice(&entry.made_by.to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&entry.flags.to_le_bytes());
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0, 0, 0, 0]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            central.extend_from_slice(&uncompressed.to_le_bytes());
            central.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&entry.external.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(&entry.name);
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out
    }

    fn write_local(
        out: &mut Vec<u8>,
        flags: u16,
        method: u16,
        crc: u32,
        payload: &[u8],
        uncompressed: u32,
        name: &[u8],
    ) {
        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        out.extend_from_slice(&20_u16.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&uncompressed.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(payload);
    }
}
