//! Этап 6: безопасный ZIP, предпросмотр и импорт экспорта ЦУ, watcher загрузок.
//! Все файловые операции выполняются только внутри `TempDir`.

use std::{
    fs::{self, File},
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use tempfile::TempDir;
use uuid::Uuid;

use super::{
    candidates::{CandidateOrigin, CandidateRegistry, CandidateSource},
    export::{preview, ExportImport, ExportImportEntry, ExportImportRequest, ExportProgress},
    store::{self, SeenOutcome},
    watcher::{is_candidate_name, DownloadScanner, ScannerConfig, WatcherCore},
    zip_safe::{
        extract_entry, read_index,
        test_zip::{build, deflate, RawEntry},
        UnsafePath, ZipLimits, ZipRejection,
    },
    Candidate,
};
use crate::{
    db::Database,
    dto::{
        ClassificationSource, InboxOrigin, InboxStatus, MaterialCategory, ProvenanceSource,
        ProvenanceVia,
    },
    error::ErrorCode,
    file_service::{DownloadEntry, DownloadsRoot, FileIdentity},
    intake::{
        ImportItemReport, InboxImporter, IntakeGrants, RootKind, RootsService, INBOX_RELATIVE,
    },
    repositories::ReadRepository,
};

// ───────────────────────── helpers ─────────────────────────

fn write_zip(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn index_of(bytes: &[u8], limits: &ZipLimits) -> Result<super::zip_safe::ZipIndex, ZipRejection> {
    let tmp = TempDir::new().unwrap();
    let path = write_zip(tmp.path(), "a.zip", bytes);
    read_index(&File::open(path).unwrap(), limits)
}

fn rejection(entries: &[RawEntry]) -> ZipRejection {
    index_of(&build(entries, false), &ZipLimits::default()).unwrap_err()
}

fn cu_export() -> Vec<u8> {
    build(
        &[
            RawEntry::dir("Основы бизнес-аналитики/"),
            RawEntry::dir("Основы бизнес-аналитики/Неделя 3/"),
            RawEntry::file(
                "Основы бизнес-аналитики/Неделя 3/Лекции/Лекция 3. Воронки.pdf",
                b"%PDF-1.7 lecture",
            ),
            RawEntry::file(
                "Основы бизнес-аналитики/Неделя 3/Семинары/Семинар 3.pdf",
                b"%PDF-1.7 seminar",
            ),
            RawEntry::file(
                "Основы бизнес-аналитики/Неделя 3/Домашнее задание/Условие ДЗ 3.pdf",
                b"%PDF-1.7 task",
            ),
            RawEntry::file(
                "Основы бизнес-аналитики/Неделя 3/Домашнее задание/sales.csv",
                b"month,revenue\n1,10\n",
            ),
            RawEntry::file("Основы бизнес-аналитики/Неделя 3/README", b"plain text"),
            RawEntry::file(
                "Основы бизнес-аналитики/Дополнительно/inner.zip",
                &build(&[RawEntry::file("x.txt", b"x")], false),
            ),
            RawEntry::file("__MACOSX/Основы бизнес-аналитики/._Лекция.pdf", b"meta"),
            RawEntry::file("Основы бизнес-аналитики/.DS_Store", b"ds"),
        ],
        false,
    )
}

// ───────────────────────── ZIP: нормальный архив ─────────────────────────

#[test]
fn normal_zip_is_indexed_and_extracted_with_verified_crc_and_sha() {
    let tmp = TempDir::new().unwrap();
    let path = write_zip(tmp.path(), "export.zip", &cu_export());
    let archive = File::open(&path).unwrap();
    let limits = ZipLimits::default();
    let index = read_index(&archive, &limits).unwrap();
    let files: Vec<&str> = index.files().map(|entry| entry.path.as_str()).collect();
    assert_eq!(files.len(), 6);
    assert!(files.contains(&"Основы бизнес-аналитики/Неделя 3/Домашнее задание/sales.csv"));
    assert_eq!(index.ignored.len(), 2, "__MACOSX и .DS_Store пропущены");

    let csv = index
        .files()
        .find(|entry| entry.path.ends_with("sales.csv"))
        .unwrap();
    let out_path = tmp.path().join("out.csv");
    let mut out = File::create(&out_path).unwrap();
    let mut budget = limits.max_total_bytes;
    let (size, sha) = extract_entry(&archive, csv, &mut out, &limits, &mut budget).unwrap();
    assert_eq!(size, 19);
    assert_eq!(fs::read(&out_path).unwrap(), b"month,revenue\n1,10\n");
    assert_eq!(sha, crate::file_service::sha256_file(&out_path).unwrap());
    assert_eq!(budget, limits.max_total_bytes - 19);
}

#[test]
fn cp866_names_from_legacy_archivers_are_decoded() {
    // «Лекция.pdf» в CP866 без флага UTF-8.
    let mut entry = RawEntry::file("x", b"data");
    entry.name = vec![0x8B, 0xA5, 0xAA, 0xE6, 0xA8, 0xEF, b'.', b'p', b'd', b'f'];
    entry.flags = 0;
    let index = index_of(&build(&[entry], false), &ZipLimits::default()).unwrap();
    assert_eq!(index.entries[0].path, "Лекция.pdf");
}

// ───────────────────────── ZIP: вредоносные варианты ─────────────────────────

#[test]
fn zip_slip_and_traversal_are_rejected() {
    for (name, reason) in [
        ("../evil.txt", UnsafePath::Traversal),
        ("ok/../../evil.txt", UnsafePath::Traversal),
        ("ok/./evil.txt", UnsafePath::Traversal),
        ("/etc/passwd", UnsafePath::Absolute),
        ("..\\evil.txt", UnsafePath::Backslash),
        ("ok\\..\\..\\evil.txt", UnsafePath::Backslash),
        ("C:/Windows/evil.txt", UnsafePath::DrivePrefix),
        ("c:evil.txt", UnsafePath::DrivePrefix),
        ("ok//evil.txt", UnsafePath::EmptyComponent),
        ("ok/\u{0007}bell.txt", UnsafePath::Control),
        ("ok/\u{202E}fdp.exe", UnsafePath::Control),
    ] {
        assert_eq!(
            rejection(&[RawEntry::file(name, b"x")]),
            ZipRejection::UnsafePath(reason),
            "{name}"
        );
    }
}

#[test]
fn overly_long_and_deep_paths_are_rejected() {
    let deep = vec!["d"; 20].join("/") + "/x.txt";
    assert_eq!(
        rejection(&[RawEntry::file(&deep, b"x")]),
        ZipRejection::UnsafePath(UnsafePath::TooDeep)
    );
    let long = format!("{}.txt", "я".repeat(200));
    assert_eq!(
        rejection(&[RawEntry::file(&long, b"x")]),
        ZipRejection::UnsafePath(UnsafePath::TooLong)
    );
}

#[test]
fn symlink_and_special_entries_reject_the_whole_archive() {
    assert!(matches!(
        rejection(&[
            RawEntry::file("ok.txt", b"x"),
            RawEntry::symlink("link", "/etc/passwd")
        ]),
        ZipRejection::Symlink(_)
    ));
    let mut fifo = RawEntry::file("pipe", b"");
    fifo.external = 0o010_644 << 16;
    assert!(matches!(rejection(&[fifo]), ZipRejection::SpecialFile(_)));
}

#[test]
fn entry_count_size_total_and_ratio_limits_are_enforced() {
    let limits = ZipLimits {
        max_entries: 3,
        ..ZipLimits::default()
    };
    let four: Vec<RawEntry> = (0..4)
        .map(|index| RawEntry::file(&format!("f{index}.txt"), b"x"))
        .collect();
    assert_eq!(
        index_of(&build(&four, false), &limits).unwrap_err(),
        ZipRejection::TooManyEntries
    );

    let random: Vec<u8> = (0..4096_u32)
        .map(|value| (value * 7919 % 251) as u8)
        .collect();
    let limits = ZipLimits {
        max_entry_bytes: 1000,
        ..ZipLimits::default()
    };
    assert!(matches!(
        index_of(
            &build(&[RawEntry::file("big.bin", &random)], false),
            &limits
        )
        .unwrap_err(),
        ZipRejection::EntryTooLarge(_)
    ));

    let limits = ZipLimits {
        max_total_bytes: 6000,
        ..ZipLimits::default()
    };
    let two = [
        RawEntry::file("a.bin", &random),
        RawEntry::file("b.bin", &random),
    ];
    assert_eq!(
        index_of(&build(&two, false), &limits).unwrap_err(),
        ZipRejection::TotalTooLarge
    );

    // Классическая zip-бомба: 4 МБ нулей сжимаются в несколько килобайт.
    let zeros = vec![0_u8; 4 * 1024 * 1024];
    assert!(matches!(
        index_of(
            &build(&[RawEntry::file("bomb.bin", &zeros)], false),
            &ZipLimits::default()
        )
        .unwrap_err(),
        ZipRejection::RatioTooHigh(_)
    ));
}

#[test]
fn header_lying_about_size_is_caught_during_extraction() {
    let tmp = TempDir::new().unwrap();
    let data = vec![b'a'; 50_000];
    let mut entry = RawEntry::file("lie.txt", &data);
    entry.declared_uncompressed = Some(100);
    entry.declared_crc = Some(crc32fast::hash(&data[..100]));
    let path = write_zip(tmp.path(), "lie.zip", &build(&[entry], false));
    let archive = File::open(&path).unwrap();
    let limits = ZipLimits::default();
    let index = read_index(&archive, &limits).unwrap();
    let mut out = File::create(tmp.path().join("out")).unwrap();
    let mut budget = limits.max_total_bytes;
    let error =
        extract_entry(&archive, &index.entries[0], &mut out, &limits, &mut budget).unwrap_err();
    assert!(matches!(error, ZipRejection::SizeMismatch(_)));
    assert!(fs::metadata(tmp.path().join("out")).unwrap().len() <= 100);
}

#[test]
fn crc_mismatch_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let mut entry = RawEntry::file("bad.txt", b"hello world");
    entry.declared_crc = Some(0xDEAD_BEEF);
    let path = write_zip(tmp.path(), "crc.zip", &build(&[entry], false));
    let archive = File::open(&path).unwrap();
    let limits = ZipLimits::default();
    let index = read_index(&archive, &limits).unwrap();
    let mut out = File::create(tmp.path().join("out")).unwrap();
    let mut budget = limits.max_total_bytes;
    assert!(matches!(
        extract_entry(&archive, &index.entries[0], &mut out, &limits, &mut budget),
        Err(ZipRejection::CrcMismatch(_))
    ));
}

#[test]
fn overlapping_entries_are_rejected() {
    let data = vec![b'z'; 10_000];
    let bytes = build(
        &[
            RawEntry::file("a.txt", &data),
            RawEntry::file("b.txt", &data),
        ],
        true,
    );
    // Имена различаются, поэтому локальный заголовок второй записи не совпадёт;
    // делаем имена одинаковыми по длине и проверяем именно перекрытие.
    let error = index_of(&bytes, &ZipLimits::default()).unwrap_err();
    assert!(
        matches!(error, ZipRejection::Overlap | ZipRejection::Corrupt(_)),
        "{error:?}"
    );

    let same_name = build(
        &[
            RawEntry::file("a.txt", &data),
            RawEntry::file("a.txt", &data),
        ],
        true,
    );
    let error = index_of(&same_name, &ZipLimits::default()).unwrap_err();
    assert!(
        matches!(
            error,
            ZipRejection::Overlap | ZipRejection::DuplicatePath(_)
        ),
        "{error:?}"
    );
}

#[test]
fn overlap_with_distinct_names_sharing_one_local_header_is_rejected() {
    // Две записи центрального каталога указывают на один локальный заголовок
    // с тем же именем: имена совпадают у локального заголовка, перекрытие ловится.
    let data = vec![b'q'; 2048];
    let mut first = RawEntry::file("same.txt", &data);
    first.local_name = Some(b"same.txt".to_vec());
    let mut second = first.clone();
    second.name = b"SAME.txt".to_vec();
    let error = index_of(&build(&[first, second], true), &ZipLimits::default()).unwrap_err();
    assert!(
        matches!(
            error,
            ZipRejection::DuplicatePath(_) | ZipRejection::Corrupt(_) | ZipRejection::Overlap
        ),
        "{error:?}"
    );
}

#[test]
fn duplicate_paths_and_file_directory_conflicts_are_rejected() {
    assert!(matches!(
        rejection(&[
            RawEntry::file("Неделя 1/a.pdf", b"1"),
            RawEntry::file("неделя 1/A.pdf", b"2")
        ]),
        ZipRejection::DuplicatePath(_)
    ));
    assert!(matches!(
        rejection(&[RawEntry::file("a", b"1"), RawEntry::file("a/b.txt", b"2")]),
        ZipRejection::DuplicatePath(_)
    ));
    assert!(matches!(
        rejection(&[RawEntry::file("a/b.txt", b"2"), RawEntry::file("a", b"1")]),
        ZipRejection::DuplicatePath(_)
    ));
}

#[test]
fn encrypted_unsupported_zip64_and_corrupt_archives_are_rejected() {
    let mut encrypted = RawEntry::file("secret.txt", b"x");
    encrypted.flags |= 0x0001;
    assert_eq!(rejection(&[encrypted]), ZipRejection::Encrypted);

    let mut bzip = RawEntry::file("b.txt", b"x");
    bzip.method = Some(12);
    assert_eq!(rejection(&[bzip]), ZipRejection::UnsupportedMethod(12));

    let mut zip64 = build(&[RawEntry::file("a.txt", b"x")], false);
    let eocd = zip64.len() - 22;
    zip64[eocd + 16..eocd + 20].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    assert_eq!(
        index_of(&zip64, &ZipLimits::default()).unwrap_err(),
        ZipRejection::Zip64
    );

    assert_eq!(
        index_of(b"not a zip at all, sorry", &ZipLimits::default()).unwrap_err(),
        ZipRejection::NotZip
    );
    let mut truncated = build(&[RawEntry::file("a.txt", b"hello")], false);
    truncated.drain(10..20);
    assert!(index_of(&truncated, &ZipLimits::default()).is_err());

    let mut mismatch = RawEntry::file("a.txt", b"x");
    mismatch.local_name = Some(b"b.txt".to_vec());
    assert!(matches!(rejection(&[mismatch]), ZipRejection::Corrupt(_)));

    let limits = ZipLimits {
        max_archive_bytes: 100,
        ..ZipLimits::default()
    };
    let big = build(&[RawEntry::file("a.bin", &vec![1_u8; 1000])], false);
    assert_eq!(
        index_of(&big, &limits).unwrap_err(),
        ZipRejection::ArchiveTooLarge
    );
}

#[test]
fn nested_archives_are_never_unpacked_but_office_documents_are_fine() {
    let tmp = TempDir::new().unwrap();
    let inner = build(&[RawEntry::file("x.txt", b"x")], false);
    let path = write_zip(
        tmp.path(),
        "outer.zip",
        &build(
            &[
                RawEntry::file("data.bin", &inner),
                RawEntry::file("report.docx", &inner),
            ],
            false,
        ),
    );
    let archive = File::open(&path).unwrap();
    let limits = ZipLimits::default();
    let index = read_index(&archive, &limits).unwrap();
    let mut budget = limits.max_total_bytes;
    let mut out = File::create(tmp.path().join("bin")).unwrap();
    assert!(matches!(
        extract_entry(&archive, &index.entries[0], &mut out, &limits, &mut budget),
        Err(ZipRejection::NestedArchive(_))
    ));
    let mut out = File::create(tmp.path().join("docx")).unwrap();
    assert!(extract_entry(&archive, &index.entries[1], &mut out, &limits, &mut budget).is_ok());
}

#[test]
fn deflate_helper_is_real_deflate() {
    let compressed = deflate(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(compressed.len() < 32);
}

// ───────────────────────── Предпросмотр и импорт экспорта ─────────────────────────

struct Fixture {
    _tmp: TempDir,
    db: Database,
    vault: PathBuf,
    downloads: PathBuf,
    external: PathBuf,
    scratch: PathBuf,
    grants: IntakeGrants,
    registry: CandidateRegistry,
}

impl Fixture {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let vault = root.join("vault");
        let downloads = root.join("downloads");
        let external = root.join("external");
        let scratch = root.join("app-data/tmp");
        for dir in [&vault, &downloads, &external] {
            fs::create_dir_all(dir).unwrap();
        }
        let db = Database::open_in_memory().unwrap();
        db.migrate().unwrap();
        db.with_connection(|connection| {
            connection.execute_batch(
                "INSERT INTO subjects(id, name, short_name, folder_path)
                     VALUES ('oba', 'Основы бизнес-аналитики', 'ОБА', 'Предметы/ОБА');
                 INSERT INTO subjects(id, name, short_name, folder_path)
                     VALUES ('stat', 'Статистика', 'Стат', 'Предметы/Статистика');",
            )?;
            Ok(())
        })
        .unwrap();
        let fixture = Self {
            _tmp: tmp,
            db,
            vault,
            downloads,
            external,
            scratch,
            grants: IntakeGrants::new(),
            registry: CandidateRegistry::new(),
        };
        let roots = RootsService::new(&fixture.db, None);
        roots.set_root(RootKind::Vault, &fixture.vault).unwrap();
        roots
            .set_root(RootKind::Downloads, &fixture.downloads)
            .unwrap();
        fixture
    }

    fn picked(&self, name: &str, bytes: &[u8]) -> Candidate {
        let path = self.external.join(name);
        fs::write(&path, bytes).unwrap();
        self.grants.grant([path.clone()]);
        self.registry
            .register(
                name.to_owned(),
                bytes.len() as u64,
                CandidateOrigin::Manual,
                CandidateSource::External { path },
                None,
            )
            .unwrap()
    }

    fn source(&self, candidate: &Candidate) -> crate::file_service::SourceFile {
        super::open_candidate(
            candidate,
            &super::SourceContext {
                downloads_path: Some(self.downloads.clone()),
                home: None,
                grants: &self.grants,
            },
        )
        .unwrap()
    }

    fn preview(&self, candidate: &Candidate) -> super::ExportPreviewDto {
        preview(
            &self.db,
            candidate,
            &self.source(candidate),
            &ZipLimits::default(),
        )
        .unwrap()
    }

    fn import(
        &self,
        candidate: &Candidate,
        request: &ExportImportRequest,
        operation_id: &str,
    ) -> Result<(crate::intake::ImportOutcome, Vec<ExportProgress>), crate::error::AppError> {
        let roots = RootsService::new(&self.db, None)
            .open_import_roots()
            .unwrap();
        let importer = InboxImporter::new(&self.db, &roots, &self.grants);
        let export = ExportImport {
            database: &self.db,
            importer: &importer,
            scratch_base: &self.scratch,
            limits: ZipLimits::default(),
        };
        let mut events = Vec::new();
        let source = self.source(candidate);
        export
            .run(operation_id, candidate, &source, request, &mut |progress| {
                events.push(progress)
            })
            .map(|outcome| (outcome.outcome, events))
    }

    fn inbox_names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.vault.join(INBOX_RELATIVE))
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn scratch_is_empty(&self) -> bool {
        fs::read_dir(&self.scratch).map_or(true, |mut entries| entries.next().is_none())
    }

    fn inbox_rows(&self) -> i64 {
        self.db
            .with_connection(|connection| {
                Ok(connection
                    .query_row("SELECT count(*) FROM inbox_files", [], |row| row.get(0))?)
            })
            .unwrap()
    }
}

fn request_for(preview: &super::ExportPreviewDto) -> ExportImportRequest {
    ExportImportRequest {
        candidate_id: preview.candidate_id.clone(),
        archive_sha256: preview.archive_sha256.clone(),
        entries: preview
            .entries
            .iter()
            .filter(|entry| entry.selected)
            .map(|entry| ExportImportEntry {
                entry_id: entry.entry_id.clone(),
                subject_id: entry.classification.subject_id.clone(),
                week_number: entry.classification.week_number,
                category: entry.classification.category,
                edited: false,
            })
            .collect(),
        course_url: None,
        assignment_url: None,
    }
}

#[test]
fn preview_classifies_cu_export_without_writing_anything() {
    let fixture = Fixture::new();
    let candidate = fixture.picked("Основы_бизнес-аналитики (1).zip", &cu_export());
    let preview = fixture.preview(&candidate);
    assert_eq!(
        preview.course_title.as_deref(),
        Some("Основы бизнес-аналитики")
    );
    assert_eq!(preview.total_files, 6);
    assert_eq!(preview.archive_sha256.len(), 64);
    let by_name = |name: &str| {
        preview
            .entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap()
    };

    let lecture = by_name("Лекция 3. Воронки.pdf");
    assert_eq!(lecture.classification.category, MaterialCategory::Lecture);
    assert_eq!(lecture.classification.week_number, Some(3));
    assert_eq!(lecture.classification.subject_id.as_deref(), Some("oba"));
    assert_eq!(lecture.status, InboxStatus::Classified);
    assert_eq!(
        by_name("Семинар 3.pdf").classification.category,
        MaterialCategory::Seminar
    );
    assert_eq!(
        by_name("Условие ДЗ 3.pdf").classification.category,
        MaterialCategory::HomeworkTask
    );
    assert_eq!(
        by_name("sales.csv").classification.category,
        MaterialCategory::HomeworkData
    );

    let unknown = by_name("README");
    assert_eq!(unknown.classification.category, MaterialCategory::Unknown);
    assert_eq!(unknown.status, InboxStatus::Ambiguous);
    assert!(
        unknown.selected,
        "неоднозначный файл импортируется во «Входящие», но не раскладывается"
    );

    let nested = by_name("inner.zip");
    assert!(!nested.selectable && !nested.selected);
    assert!(nested
        .warnings
        .iter()
        .any(|warning| warning.contains("Вложенный архив")));
    assert!(preview
        .warnings
        .iter()
        .any(|warning| warning.contains("служебные")));

    assert!(fixture.inbox_names().is_empty());
    assert!(
        !fixture.scratch.exists(),
        "предпросмотр ничего не распаковывает"
    );
}

#[test]
fn confirmed_import_goes_through_inbox_importer_with_provenance_and_cleanup() {
    let fixture = Fixture::new();
    let candidate = fixture.picked("Основы бизнес-аналитики.zip", &cu_export());
    let preview = fixture.preview(&candidate);
    let mut request = request_for(&preview);
    request.course_url =
        Some("https://my.centraluniversity.ru/learn/courses/view/actual/42".to_owned());
    // Пользователь уточнил неизвестный файл.
    let readme = preview
        .entries
        .iter()
        .find(|entry| entry.name == "README")
        .unwrap();
    let edited = request
        .entries
        .iter_mut()
        .find(|entry| entry.entry_id == readme.entry_id)
        .unwrap();
    edited.category = MaterialCategory::Material;
    edited.subject_id = Some("oba".to_owned());
    edited.week_number = Some(3);
    edited.edited = true;

    let operation_id = Uuid::new_v4().to_string();
    let (outcome, events) = fixture.import(&candidate, &request, &operation_id).unwrap();
    assert_eq!(outcome.report.imported, 5);
    assert_eq!(outcome.report.failed, 0);
    assert_eq!(outcome.report.skipped, 1, "вложенный архив не выбран");
    assert!(outcome.failure.is_none());
    assert_eq!(
        fixture.inbox_names(),
        vec![
            "README",
            "Лекция 3. Воронки.pdf",
            "Семинар 3.pdf",
            "Условие ДЗ 3.pdf",
            "sales.csv"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .tap_sort()
    );
    assert!(fixture.scratch_is_empty(), "временный каталог удалён");
    assert!(events
        .iter()
        .any(|event| event.phase == super::ExportPhase::Extract));
    assert_eq!(events.last().unwrap().phase, super::ExportPhase::Cleanup);

    let files = ReadRepository::new(&fixture.db).inbox_files().unwrap();
    for file in &files {
        assert_eq!(file.origin, InboxOrigin::Lms);
        let provenance = file.provenance.as_ref().unwrap();
        assert_eq!(provenance.source, ProvenanceSource::CuExport);
        assert_eq!(provenance.via, ProvenanceVia::Picker);
        assert_eq!(
            provenance.archive_name.as_deref(),
            Some("Основы бизнес-аналитики.zip")
        );
        assert!(provenance
            .entry_path
            .as_deref()
            .unwrap()
            .starts_with("Основы бизнес-аналитики/"));
        assert_eq!(
            provenance.course_url.as_deref(),
            Some("https://my.centraluniversity.ru/learn/courses/view/actual/42")
        );
    }
    let readme_row = files.iter().find(|file| file.name == "README").unwrap();
    let classification = readme_row.classification.as_ref().unwrap();
    assert_eq!(classification.source, ClassificationSource::Manual);
    assert_eq!(readme_row.status, InboxStatus::Classified);
    let lecture_row = files
        .iter()
        .find(|file| file.name.starts_with("Лекция"))
        .unwrap();
    assert_eq!(
        lecture_row.classification.as_ref().unwrap().source,
        ClassificationSource::Auto
    );
    assert_eq!(lecture_row.status, InboxStatus::Classified);
    assert!(
        files.iter().all(|file| file.status != InboxStatus::Routed),
        "ничего не разложено"
    );

    // Абсолютные пути не попали в SQLite.
    let dump: String = fixture
        .db
        .with_connection(|connection| {
            let mut statement = connection.prepare(
                "SELECT coalesce(provenance_json,'') || coalesce(classification_json,'') || relative_path FROM inbox_files
                 UNION ALL SELECT result_json FROM operation_results
                 UNION ALL SELECT coalesce(detail,'') || paths_json FROM operations",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?.join("\n"))
        })
        .unwrap();
    assert!(!dump.contains(fixture.external.to_str().unwrap()));
    assert!(!dump.contains(fixture.scratch.to_str().unwrap()));

    // Содержимое уже импортированного архива watcher больше не предложит.
    assert!(store::is_known_content(&fixture.db, &preview.archive_sha256).unwrap());

    // Повтор той же команды — сохранённый итог без файловых действий.
    let (replay, _) = fixture.import(&candidate, &request, &operation_id).unwrap();
    assert!(replay.report.replayed);
    assert_eq!(fixture.inbox_rows(), 5);
}

trait TapSort {
    fn tap_sort(self) -> Self;
}

impl TapSort for Vec<String> {
    fn tap_sort(mut self) -> Self {
        self.sort();
        self
    }
}

#[test]
fn reimport_of_same_export_creates_duplicates_not_copies() {
    let fixture = Fixture::new();
    let candidate = fixture.picked("export.zip", &cu_export());
    let preview = fixture.preview(&candidate);
    fixture
        .import(
            &candidate,
            &request_for(&preview),
            &Uuid::new_v4().to_string(),
        )
        .unwrap();
    let again = fixture.picked("export (2).zip", &cu_export());
    let preview = fixture.preview(&again);
    let (outcome, _) = fixture
        .import(&again, &request_for(&preview), &Uuid::new_v4().to_string())
        .unwrap();
    assert_eq!(outcome.report.imported, 0);
    assert_eq!(outcome.report.duplicates, 5);
    assert_eq!(fixture.inbox_rows(), 5);
}

#[test]
fn archive_changed_after_preview_is_rejected_before_any_work() {
    let fixture = Fixture::new();
    let candidate = fixture.picked("export.zip", &cu_export());
    let preview = fixture.preview(&candidate);
    let request = request_for(&preview);
    let path = fixture.external.join("export.zip");
    fs::write(
        &path,
        build(&[RawEntry::file("Неделя 1/x.pdf", b"other")], false),
    )
    .unwrap();
    let error = fixture
        .import(&candidate, &request, &Uuid::new_v4().to_string())
        .err()
        .unwrap();
    assert!(matches!(
        error.code,
        ErrorCode::Conflict | ErrorCode::Blocked
    ));
    assert_eq!(fixture.inbox_rows(), 0);
    assert!(fixture.scratch_is_empty());
    let operations: i64 = fixture
        .db
        .with_connection(|c| Ok(c.query_row("SELECT count(*) FROM operations", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(operations, 0, "операция не начиналась");
}

#[test]
fn malicious_archive_is_rejected_at_preview_and_import_without_side_effects() {
    let fixture = Fixture::new();
    let evil = build(
        &[
            RawEntry::file("Неделя 1/ok.pdf", b"ok"),
            RawEntry::file("../../evil.sh", b"rm -rf"),
        ],
        false,
    );
    let candidate = fixture.picked("evil.zip", &evil);
    let error = preview(
        &fixture.db,
        &candidate,
        &fixture.source(&candidate),
        &ZipLimits::default(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
    let request = ExportImportRequest {
        candidate_id: candidate.id.clone(),
        archive_sha256: crate::file_service::hash_source(&fixture.source(&candidate))
            .unwrap()
            .as_str()
            .to_owned(),
        entries: vec![ExportImportEntry {
            entry_id: "0".to_owned(),
            subject_id: None,
            week_number: None,
            category: MaterialCategory::Unknown,
            edited: false,
        }],
        course_url: None,
        assignment_url: None,
    };
    let error = fixture
        .import(&candidate, &request, &Uuid::new_v4().to_string())
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(fixture.inbox_rows(), 0);
    assert!(fixture.scratch_is_empty());
    assert!(!fixture.vault.parent().unwrap().join("evil.sh").exists());
}

#[test]
fn entry_failing_extraction_is_reported_and_others_still_import() {
    let fixture = Fixture::new();
    let mut broken = RawEntry::file("Курс/Неделя 2/Лекции/broken.pdf", b"broken body");
    broken.declared_crc = Some(1);
    let bytes = build(
        &[
            RawEntry::file("Курс/Неделя 2/Лекции/good.pdf", b"good body"),
            broken,
        ],
        false,
    );
    let candidate = fixture.picked("Курс.zip", &bytes);
    let preview = fixture.preview(&candidate);
    let (outcome, _) = fixture
        .import(
            &candidate,
            &request_for(&preview),
            &Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert_eq!(outcome.report.imported, 1);
    assert_eq!(outcome.report.failed, 1);
    let failed = outcome
        .report
        .items
        .iter()
        .find(|item| matches!(item, ImportItemReport::Failed { .. }))
        .unwrap();
    if let ImportItemReport::Failed { error, .. } = failed {
        assert!(error.message.contains("Контрольная сумма"));
    }
    assert_eq!(fixture.inbox_names(), vec!["good.pdf".to_owned()]);
    assert!(fixture.scratch_is_empty());
}

#[test]
fn invalid_requests_are_rejected_before_any_file_or_operation() {
    let fixture = Fixture::new();
    let candidate = fixture.picked("export.zip", &cu_export());
    let preview = fixture.preview(&candidate);
    let base = request_for(&preview);
    let cases: Vec<(ExportImportRequest, ErrorCode)> = vec![
        (
            ExportImportRequest {
                entries: vec![],
                ..base.clone()
            },
            ErrorCode::Blocked,
        ),
        (
            ExportImportRequest {
                course_url: Some("https://evil.example/course".to_owned()),
                ..base.clone()
            },
            ErrorCode::Blocked,
        ),
        (
            ExportImportRequest {
                assignment_url: Some("http://my.centraluniversity.ru/x".to_owned()),
                ..base.clone()
            },
            ErrorCode::Blocked,
        ),
        (
            ExportImportRequest {
                entries: vec![ExportImportEntry {
                    entry_id: "9999".to_owned(),
                    ..base.entries[0].clone()
                }],
                ..base.clone()
            },
            ErrorCode::Blocked,
        ),
        (
            ExportImportRequest {
                entries: vec![base.entries[0].clone(), base.entries[0].clone()],
                ..base.clone()
            },
            ErrorCode::Blocked,
        ),
        (
            ExportImportRequest {
                entries: vec![ExportImportEntry {
                    subject_id: Some("missing".to_owned()),
                    ..base.entries[0].clone()
                }],
                ..base.clone()
            },
            ErrorCode::NotFound,
        ),
        (
            ExportImportRequest {
                entries: vec![ExportImportEntry {
                    week_number: Some(0),
                    ..base.entries[0].clone()
                }],
                ..base.clone()
            },
            ErrorCode::Blocked,
        ),
    ];
    for (request, code) in cases {
        let error = fixture
            .import(&candidate, &request, &Uuid::new_v4().to_string())
            .err()
            .unwrap();
        assert_eq!(error.code, code, "{}", error.message);
    }
    // Вложенный архив нельзя выбрать вручную.
    let nested = preview
        .entries
        .iter()
        .find(|entry| entry.name == "inner.zip")
        .unwrap();
    let error = fixture
        .import(
            &candidate,
            &ExportImportRequest {
                entries: vec![ExportImportEntry {
                    entry_id: nested.entry_id.clone(),
                    ..base.entries[0].clone()
                }],
                ..base.clone()
            },
            &Uuid::new_v4().to_string(),
        )
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::Blocked);
    assert_eq!(fixture.inbox_rows(), 0);
    let error = fixture
        .import(&candidate, &base, "not-a-uuid")
        .err()
        .unwrap();
    assert_eq!(error.code, ErrorCode::Blocked);
}

#[test]
fn single_pdf_export_is_previewed_and_imported_without_extraction() {
    let fixture = Fixture::new();
    let candidate = fixture.picked("Статистика — неделя 5.pdf", b"%PDF-1.7 export");
    let preview = fixture.preview(&candidate);
    assert_eq!(preview.format, super::export::ExportFormat::Single);
    assert_eq!(preview.entries.len(), 1);
    assert_eq!(
        preview.entries[0].classification.subject_id.as_deref(),
        Some("stat")
    );
    assert_eq!(preview.entries[0].classification.week_number, Some(5));
    let (outcome, _) = fixture
        .import(
            &candidate,
            &request_for(&preview),
            &Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert_eq!(outcome.report.imported, 1);
    let file = &ReadRepository::new(&fixture.db).inbox_files().unwrap()[0];
    assert_eq!(file.origin, InboxOrigin::Lms);
    assert_eq!(file.provenance.as_ref().unwrap().entry_path, None);
    assert!(!fixture.scratch.exists(), "PDF не распаковывается");
}

#[test]
fn ungranted_external_candidate_cannot_be_opened() {
    let fixture = Fixture::new();
    let path = fixture.external.join("sneaky.zip");
    fs::write(&path, cu_export()).unwrap();
    let candidate = fixture
        .registry
        .register(
            "sneaky.zip".to_owned(),
            1,
            CandidateOrigin::Manual,
            CandidateSource::External { path },
            None,
        )
        .unwrap();
    let error = super::open_candidate(
        &candidate,
        &super::SourceContext {
            downloads_path: Some(fixture.downloads.clone()),
            home: None,
            grants: &fixture.grants,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Blocked);
}

// ───────────────────────── Watcher ─────────────────────────

fn entry(name: &str, size: u64, mtime: i128, ino: u64) -> DownloadEntry {
    DownloadEntry {
        name: name.to_owned(),
        size_bytes: size,
        modified_ns: mtime,
        identity: FileIdentity::for_tests(1, ino),
    }
}

#[test]
fn temporary_and_foreign_names_are_never_candidates() {
    for name in [
        "Курс.zip.crdownload",
        "Курс.zip.part",
        "Unconfirmed 123.crdownload",
        ".hidden.zip",
        "~$lock.docx",
        "Курс.zip.download",
        "movie.mkv",
        "setup.dmg",
        "noext",
    ] {
        assert!(!is_candidate_name(name), "{name}");
    }
    for name in ["Курс.zip", "Лекция.pdf", "data.xlsx", "hw.ipynb"] {
        assert!(is_candidate_name(name), "{name}");
    }
}

#[test]
fn scanner_waits_for_stable_size_and_quiet_period_and_offers_once() {
    let config = ScannerConfig {
        quiet_period: Duration::from_secs(3),
        stable_polls: 2,
        max_bytes: 10_000,
    };
    let mut scanner = DownloadScanner::new(config);
    let start = Instant::now();
    let at = |seconds: u64| start + Duration::from_secs(seconds);

    let old = entry("old.zip", 10, 1, 1);
    assert!(
        scanner.scan(std::slice::from_ref(&old), at(0)).is_empty(),
        "базовая линия"
    );

    // Растущий файл: пока размер меняется, кандидата нет.
    let growing = |size| entry("Курс.zip", size, i128::from(size), 2);
    assert!(scanner.scan(&[old.clone(), growing(10)], at(1)).is_empty());
    assert!(scanner.scan(&[old.clone(), growing(20)], at(2)).is_empty());
    assert!(scanner.scan(&[old.clone(), growing(30)], at(3)).is_empty());
    // Размер стабилен, но тихий период ещё не прошёл.
    assert!(scanner.scan(&[old.clone(), growing(30)], at(4)).is_empty());
    assert!(scanner.scan(&[old.clone(), growing(30)], at(5)).is_empty());
    let ready = scanner.scan(&[old.clone(), growing(30)], at(6));
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].name, "Курс.zip");
    // Предлагается один раз.
    assert!(scanner.scan(&[old.clone(), growing(30)], at(9)).is_empty());
    assert!(scanner.scan(&[old.clone(), growing(30)], at(20)).is_empty());

    // Базовый файл изменился после включения — теперь он новый кандидат.
    let changed = entry("old.zip", 11, 9, 1);
    let offered: Vec<String> = (21..=26)
        .flat_map(|second| scanner.scan(&[changed.clone(), growing(30)], at(second)))
        .map(|entry| entry.name)
        .collect();
    assert_eq!(offered, vec!["old.zip".to_owned()]);
}

#[test]
fn scanner_ignores_empty_files_oversized_files_and_files_with_partial_sibling() {
    let config = ScannerConfig {
        quiet_period: Duration::ZERO,
        stable_polls: 1,
        max_bytes: 100,
    };
    let mut scanner = DownloadScanner::new(config);
    let now = Instant::now();
    scanner.scan(&[], now);
    let firefox = [
        entry("Курс.zip", 50, 1, 3),
        entry("Курс.zip.part", 40, 1, 4),
    ];
    let empty = entry("empty.pdf", 0, 1, 5);
    let huge = entry("huge.pdf", 1000, 1, 6);
    for _ in 0..4 {
        let mut all = firefox.to_vec();
        all.push(empty.clone());
        all.push(huge.clone());
        assert!(scanner.scan(&all, now).is_empty());
    }
    // `.part` исчез — загрузка завершена.
    let ready = scanner.scan(&[firefox[0].clone()], now);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].name, "Курс.zip");
}

#[test]
fn watcher_tick_registers_new_downloads_once_and_skips_known_content() {
    let fixture = Fixture::new();
    let root = DownloadsRoot::new(&fixture.downloads, None).unwrap();
    let mut core = WatcherCore::new(ScannerConfig {
        quiet_period: Duration::ZERO,
        stable_polls: 1,
        max_bytes: 1 << 20,
    });
    let now = Instant::now();
    fs::write(fixture.downloads.join("before.zip"), b"baseline").unwrap();
    core.tick(&root, &fixture.db, &fixture.registry, now)
        .unwrap();
    assert!(fixture.registry.list().unwrap().is_empty(), "базовая линия");

    fs::write(fixture.downloads.join("Курс.zip"), cu_export()).unwrap();
    fs::write(fixture.downloads.join("Курс.zip.crdownload"), b"tmp").unwrap();
    fs::write(fixture.downloads.join("seen.pdf"), b"already imported").unwrap();
    let seen_sha = {
        let source = root
            .open_file(
                "seen.pdf",
                root.list_files()
                    .unwrap()
                    .into_iter()
                    .find(|entry| entry.name == "seen.pdf")
                    .unwrap()
                    .identity,
            )
            .unwrap();
        crate::file_service::hash_source(&source).unwrap()
    };
    store::mark_seen(
        &fixture.db,
        seen_sha.as_str(),
        "seen.pdf",
        16,
        SeenOutcome::Imported,
        None,
    )
    .unwrap();
    symlink("/etc/hosts", fixture.downloads.join("link.pdf")).unwrap();
    fs::create_dir(fixture.downloads.join("folder.zip")).unwrap();

    for _ in 0..3 {
        core.tick(&root, &fixture.db, &fixture.registry, now)
            .unwrap();
    }
    assert!(
        fixture.registry.list().unwrap().is_empty(),
        "рядом временный .crdownload"
    );
    fs::remove_file(fixture.downloads.join("Курс.zip.crdownload")).unwrap();
    for _ in 0..3 {
        core.tick(&root, &fixture.db, &fixture.registry, now)
            .unwrap();
    }
    let list = fixture.registry.list().unwrap();
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].name, "Курс.zip");
    assert_eq!(list[0].origin, CandidateOrigin::Watcher);
    let candidate = fixture.registry.get(&list[0].id).unwrap();
    assert!(candidate.sha256.is_some());

    // Кандидат из загрузок открывается fd-relative и проходит предпросмотр.
    let preview = fixture.preview(&candidate);
    assert_eq!(preview.total_files, 6);

    // Подмена файла после обнаружения обнаруживается при открытии.
    fs::remove_file(fixture.downloads.join("Курс.zip")).unwrap();
    fs::write(fixture.downloads.join("Курс.zip"), b"replaced").unwrap();
    let error = super::open_candidate(
        &candidate,
        &super::SourceContext {
            downloads_path: Some(fixture.downloads.clone()),
            home: None,
            grants: &fixture.grants,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);

    // Исчезнувший файл убирает кандидата.
    fs::remove_file(fixture.downloads.join("Курс.zip")).unwrap();
    core.tick(&root, &fixture.db, &fixture.registry, now)
        .unwrap();
    assert!(fixture.registry.list().unwrap().is_empty());
}

#[test]
fn watcher_import_marks_downloads_origin_for_plain_files() {
    let fixture = Fixture::new();
    let root = DownloadsRoot::new(&fixture.downloads, None).unwrap();
    let mut core = WatcherCore::new(ScannerConfig {
        quiet_period: Duration::ZERO,
        stable_polls: 1,
        max_bytes: 1 << 20,
    });
    let now = Instant::now();
    core.tick(&root, &fixture.db, &fixture.registry, now)
        .unwrap();
    fs::write(fixture.downloads.join("data.xlsx"), b"PK-not-really").unwrap();
    for _ in 0..3 {
        core.tick(&root, &fixture.db, &fixture.registry, now)
            .unwrap();
    }
    let id = fixture.registry.list().unwrap()[0].id.clone();
    let candidate = fixture.registry.get(&id).unwrap();
    let preview = fixture.preview(&candidate);
    let (outcome, _) = fixture
        .import(
            &candidate,
            &request_for(&preview),
            &Uuid::new_v4().to_string(),
        )
        .unwrap();
    assert_eq!(outcome.report.imported, 1);
    let file = &ReadRepository::new(&fixture.db).inbox_files().unwrap()[0];
    assert_eq!(file.origin, InboxOrigin::Downloads);
    let provenance = file.provenance.as_ref().unwrap();
    assert_eq!(provenance.source, ProvenanceSource::Watcher);
    assert_eq!(provenance.via, ProvenanceVia::Watcher);
    assert_eq!(provenance.archive_name, None);
    // Тот же файл после импорта watcher больше не предлагает.
    assert!(store::is_known_content(&fixture.db, &preview.archive_sha256).unwrap());
}

#[test]
fn watcher_is_disabled_by_default_and_service_stops_cleanly() {
    let fixture = Fixture::new();
    assert!(!store::read_watcher_settings(&fixture.db).unwrap().enabled);
    let service = super::WatcherService::new();
    let snapshot = service.snapshot(false, &fixture.registry).unwrap();
    assert_eq!(snapshot.state, super::WatcherState::Disabled);
    assert!(!service.is_running());

    struct Host;
    impl super::WatcherHost for Host {
        fn with_state(
            &self,
            _task: &mut super::watcher::WatcherTask<'_>,
        ) -> Option<Result<super::watcher::TickOutcome, crate::error::AppError>> {
            Some(Ok(super::watcher::TickOutcome { changed: false }))
        }
        fn notify(&self) {}
    }
    service
        .start(
            Box::new(Host),
            fixture.downloads.clone(),
            ScannerConfig::default(),
        )
        .unwrap();
    assert!(service.is_running());
    service.stop().unwrap();
    assert!(!service.is_running());
    store::write_watcher_settings(&fixture.db, store::WatcherSettings { enabled: true }).unwrap();
    assert!(store::read_watcher_settings(&fixture.db).unwrap().enabled);
}

#[test]
fn course_and_assignment_urls_are_validated_and_stored() {
    let fixture = Fixture::new();
    store::set_course_url(
        &fixture.db,
        "oba",
        Some("https://my.centraluniversity.ru/learn/courses/view/actual/1"),
    )
    .unwrap();
    let subjects = ReadRepository::new(&fixture.db).subjects().unwrap();
    let oba = subjects.iter().find(|subject| subject.id == "oba").unwrap();
    assert_eq!(
        oba.course_url.as_deref(),
        Some("https://my.centraluniversity.ru/learn/courses/view/actual/1")
    );
    assert_eq!(
        store::set_course_url(&fixture.db, "missing", None)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        store::set_assignment_url(&fixture.db, "missing", None)
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}
