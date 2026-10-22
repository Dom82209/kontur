//! PPTX: заголовки и текст слайдов в порядке показа, таблицы на слайдах и
//! заметки докладчика (если есть). Изображения, видео, диаграммы и встроенные
//! объекты не извлекаются.

use super::{
    model::{
        clean_text, Block, Budget, Collector, ExtractError, ExtractResult, Extracted,
        ExtractionLimits, SourceFormat, WarningCode,
    },
    ooxml::{attr, flag, name_of, rel_attr, Node, Package, Xml},
};

pub fn extract(
    bytes: &[u8],
    limits: &ExtractionLimits,
    budget: &Budget,
) -> ExtractResult<Extracted> {
    let mut package = Package::open(bytes, limits, budget)?;
    let mut collector = Collector::new(limits);
    let mut metadata = package.properties()?;
    package.package_warnings(&mut collector);

    let presentation_path = package.main_part("ppt/presentation.xml")?;
    let relationships = package.relationships(&presentation_path)?;
    let presentation = package
        .read_xml(&presentation_path)?
        .ok_or_else(|| ExtractError::Corrupt("В PPTX нет описания презентации".into()))?;
    let slide_ids = slide_order(&presentation, budget, limits)?;
    metadata.slide_count = Some(slide_ids.len().min(u32::MAX as usize) as u32);

    let mut images = 0_usize;
    let mut media = 0_usize;
    for (position, rel_id) in slide_ids.iter().enumerate() {
        budget.check()?;
        if collector.is_full() {
            break;
        }
        let number = (position + 1) as u32;
        if position >= limits.max_slides {
            collector.warn(
                WarningCode::SlidesTruncated,
                format!(
                    "Извлечены первые {} слайдов из {}",
                    limits.max_slides,
                    slide_ids.len()
                ),
                None,
            );
            break;
        }
        let location = Some(format!("слайд {number}"));
        let Some(path) = relationships
            .iter()
            .find(|relationship| &relationship.id == rel_id && relationship.kind == "slide")
            .and_then(|relationship| relationship.target.clone())
        else {
            collector.warn(
                WarningCode::PartUnreadable,
                "Слайд не найден в файле",
                location,
            );
            continue;
        };
        let xml = match package.read_xml(&path) {
            Ok(Some(xml)) => xml,
            Ok(None) => {
                collector.warn(
                    WarningCode::PartUnreadable,
                    "Слайд отсутствует в файле",
                    location,
                );
                continue;
            }
            Err(error @ (ExtractError::Interrupted(_) | ExtractError::Rejected(_))) => {
                return Err(error)
            }
            Err(_) => {
                collector.warn(
                    WarningCode::PartUnreadable,
                    "Слайд повреждён — пропущен",
                    location,
                );
                continue;
            }
        };
        let slide = parse_shapes(&xml, budget, limits, &[])?;
        images += slide.images;
        media += slide.media;
        let slide_relationships = package.relationships(&path)?;
        if slide_relationships
            .iter()
            .any(|relationship| relationship.kind == "chart")
        {
            collector.warn(
                WarningCode::ChartsSkipped,
                "Диаграммы на слайдах не извлекаются",
                None,
            );
        }
        let notes = match slide_relationships
            .iter()
            .find(|relationship| relationship.kind == "notesSlide")
            .and_then(|relationship| relationship.target.clone())
        {
            Some(notes_path) => match package.optional_xml(&notes_path)? {
                Some(notes_xml) => {
                    let notes = parse_shapes(
                        &notes_xml,
                        budget,
                        limits,
                        &["sldImg", "sldNum", "hdr", "ftr", "dt"],
                    )?;
                    let text = notes.body.join("\n");
                    (!text.trim().is_empty()).then_some(text)
                }
                None => None,
            },
            None => None,
        };
        collector.push(Block::Slide {
            number,
            title: slide.title,
            body: slide.body,
            notes,
            hidden: slide.hidden,
        });
    }
    if images > 0 {
        collector.warn(
            WarningCode::ImagesSkipped,
            format!("Изображения на слайдах не извлекаются ({images})"),
            None,
        );
    }
    if media > 0 {
        collector.warn(
            WarningCode::EmbeddedObjectsSkipped,
            format!("Видео, аудио и встроенные объекты не извлекаются ({media})"),
            None,
        );
    }
    let mut extracted = collector.finish(SourceFormat::Pptx, metadata);
    if extracted.text.is_empty() {
        extracted.warnings.push(super::model::ExtractionWarning {
            code: WarningCode::EmptyDocument,
            message: "В презентации нет текста".into(),
            location: None,
        });
    }
    Ok(extracted)
}

fn slide_order(
    xml: &str,
    budget: &Budget,
    limits: &ExtractionLimits,
) -> ExtractResult<Vec<String>> {
    let mut reader = Xml::new(xml, budget, limits);
    let mut ids = Vec::new();
    let mut in_list = false;
    loop {
        match reader.next_node()? {
            Node::Start(start) if name_of(&start) == "sldIdLst" => in_list = true,
            Node::Start(start) | Node::Empty(start) if in_list && name_of(&start) == "sldId" => {
                if let Some(id) = rel_attr(&start, "id") {
                    ids.push(id);
                }
            }
            Node::End(name) if name == "sldIdLst" => in_list = false,
            Node::Eof => break,
            _ => {}
        }
    }
    Ok(ids)
}

#[derive(Default)]
struct SlideText {
    title: Option<String>,
    body: Vec<String>,
    hidden: bool,
    images: usize,
    media: usize,
}

#[derive(Default)]
struct Shape {
    placeholder: Option<String>,
    paragraphs: Vec<String>,
}

/// Текст фигур слайда. `skip_placeholders` — типы заполнителей, которые не
/// являются содержимым (номер слайда, миниатюра слайда в заметках).
fn parse_shapes(
    xml: &str,
    budget: &Budget,
    limits: &ExtractionLimits,
    skip_placeholders: &[&str],
) -> ExtractResult<SlideText> {
    let mut reader = Xml::new(xml, budget, limits);
    let mut slide = SlideText::default();
    let mut shape: Option<Shape> = None;
    let mut paragraph: Option<String> = None;
    let mut in_text = false;
    let mut row: Option<Vec<String>> = None;
    let mut cell: Option<String> = None;
    loop {
        match reader.next_node()? {
            Node::Start(start) => match name_of(&start) {
                "sld" => slide.hidden = !flag(&start, "show", true),
                "Fallback" => reader.skip_subtree()?,
                "sp" => shape = Some(Shape::default()),
                "pic" => slide.images += 1,
                "graphicFrame" => {}
                "tr" => row = Some(Vec::new()),
                "tc" => cell = Some(String::new()),
                "p" => paragraph = Some(String::new()),
                "t" => in_text = true,
                "ph" => set_placeholder(&mut shape, &start),
                "videoFile" | "audioFile" | "oleObj" => slide.media += 1,
                _ => {}
            },
            Node::Empty(start) => match name_of(&start) {
                "sld" => slide.hidden = !flag(&start, "show", true),
                "ph" => set_placeholder(&mut shape, &start),
                "br" => {
                    if let Some(text) = paragraph.as_mut() {
                        text.push('\n');
                    }
                }
                "videoFile" | "audioFile" | "oleObj" => slide.media += 1,
                _ => {}
            },
            Node::Text(text) => {
                if in_text {
                    if let Some(current) = paragraph.as_mut() {
                        current.push_str(&text);
                    }
                }
            }
            Node::End(name) => match name.as_str() {
                "t" => in_text = false,
                "p" => {
                    let Some(text) = paragraph.take() else {
                        continue;
                    };
                    let text = clean_text(text.trim());
                    if text.trim().is_empty() {
                        continue;
                    }
                    if let Some(cell) = cell.as_mut() {
                        if !cell.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(&text);
                    } else if let Some(shape) = shape.as_mut() {
                        shape.paragraphs.push(text);
                    } else {
                        slide.body.push(text);
                    }
                }
                "tc" => {
                    if let (Some(row), Some(text)) = (row.as_mut(), cell.take()) {
                        row.push(text);
                    }
                }
                "tr" => {
                    if let Some(row) = row.take() {
                        if row.iter().any(|cell| !cell.is_empty()) {
                            slide.body.push(format!("| {} |", row.join(" | ")));
                        }
                    }
                }
                "sp" => {
                    let Some(finished) = shape.take() else {
                        continue;
                    };
                    let placeholder = finished.placeholder.as_deref().unwrap_or("");
                    if skip_placeholders.contains(&placeholder) {
                        continue;
                    }
                    if matches!(placeholder, "title" | "ctrTitle") && slide.title.is_none() {
                        let title = finished.paragraphs.join(" ");
                        if !title.trim().is_empty() {
                            slide.title = Some(title);
                        }
                    } else {
                        slide.body.extend(finished.paragraphs);
                    }
                }
                _ => {}
            },
            Node::Eof => break,
        }
    }
    Ok(slide)
}

fn set_placeholder(shape: &mut Option<Shape>, start: &quick_xml::events::BytesStart<'_>) {
    if let Some(shape) = shape.as_mut() {
        shape.placeholder = Some(attr(start, "type").unwrap_or_else(|| "body".into()));
    }
}
