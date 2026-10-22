//! Изображения: формат и размеры по заголовку файла; текст — только через
//! явно включённый локальный OCR.

use super::{
    model::{
        clean_text, Block, Budget, Collector, ExtractError, ExtractResult, Extracted,
        ExtractionLimits, ExtractionMetadata, ExtractionStatus, SourceFormat, WarningCode,
    },
    ocr::{OcrEngine, OcrError},
};

/// Как поступать с OCR при разборе изображения.
pub enum OcrMode<'a> {
    Disabled,
    Unavailable,
    Enabled(&'a dyn OcrEngine),
}

pub fn image_format(bytes: &[u8]) -> Option<&'static str> {
    use imagesize::ImageType;
    Some(match imagesize::image_type(bytes).ok()? {
        ImageType::Png => "png",
        ImageType::Jpeg => "jpeg",
        ImageType::Gif => "gif",
        ImageType::Webp => "webp",
        ImageType::Tiff => "tiff",
        ImageType::Bmp => "bmp",
        ImageType::Heif(_) => "heif",
        _ => return None,
    })
}

fn ocr_supported(format: &str) -> bool {
    matches!(format, "png" | "jpeg" | "tiff" | "bmp" | "webp" | "gif")
}

pub fn extract(
    bytes: &[u8],
    limits: &ExtractionLimits,
    budget: &Budget,
    ocr: OcrMode<'_>,
) -> ExtractResult<Extracted> {
    let format = image_format(bytes)
        .ok_or_else(|| ExtractError::Unsupported("Формат изображения не распознан".into()))?;
    let size = imagesize::blob_size(bytes)
        .map_err(|_| ExtractError::Corrupt("Заголовок изображения повреждён".into()))?;
    let metadata = ExtractionMetadata {
        image_format: Some(format.to_owned()),
        image_width: u32::try_from(size.width).ok(),
        image_height: u32::try_from(size.height).ok(),
        ..ExtractionMetadata::default()
    };
    let needs_ocr = |message: &str, warning: Option<(WarningCode, &str)>| {
        let mut extracted =
            Extracted::empty(SourceFormat::Image, ExtractionStatus::NeedsOcr, message);
        extracted.metadata = metadata.clone();
        if let Some((code, text)) = warning {
            extracted.warnings.push(super::model::ExtractionWarning {
                code,
                message: text.to_owned(),
                location: None,
            });
        }
        Ok(extracted)
    };
    let engine = match ocr {
        OcrMode::Disabled => return needs_ocr(
            "Текст на изображении не распознаётся: OCR выключен в Настройках. Метаданные сохранены",
            Some((WarningCode::OcrDisabled, "OCR выключен")),
        ),
        OcrMode::Unavailable => {
            return needs_ocr(
                "OCR включён, но локальный Tesseract не найден. Метаданные сохранены",
                Some((WarningCode::OcrUnavailable, "Локальный OCR недоступен")),
            )
        }
        OcrMode::Enabled(engine) => engine,
    };
    if !ocr_supported(format) {
        return needs_ocr(
            "Этот формат изображения OCR не поддерживает — сохраните как PNG или JPEG",
            Some((WarningCode::OcrUnavailable, "Формат не поддерживается OCR")),
        );
    }
    budget.check()?;
    match engine.recognize(bytes, budget) {
        Ok(text) => {
            let text = clean_text(&text);
            let mut collector = Collector::new(limits);
            for paragraph in text.split("\n\n") {
                let paragraph = paragraph.trim();
                if !paragraph.is_empty() {
                    collector.push(Block::Paragraph {
                        text: paragraph.to_owned(),
                    });
                }
            }
            let mut extracted = collector.finish(SourceFormat::Image, metadata.clone());
            extracted.ocr_used = true;
            extracted.status_message = Some(format!(
                "Текст распознан локально ({}, {}). Проверьте результат: OCR может ошибаться",
                engine.name(),
                engine.languages()
            ));
            if extracted.text.is_empty() {
                extracted.warnings.push(super::model::ExtractionWarning {
                    code: WarningCode::EmptyDocument,
                    message: "OCR не нашёл текста на изображении".into(),
                    location: None,
                });
            }
            Ok(extracted)
        }
        Err(OcrError::Interrupted(interrupt)) => Err(ExtractError::Interrupted(interrupt)),
        Err(OcrError::Failed(message)) => needs_ocr(
            &format!("{message}. Метаданные сохранены"),
            Some((WarningCode::OcrFailed, "Распознавание не удалось")),
        ),
    }
}
