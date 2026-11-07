//! Этап 9: объяснимые признаки письменного стиля.
//!
//! Всё считается локально по тексту — без сети и без моделей. Это эвристика:
//! она показывает, чем текст отличается от подтверждённых работ пользователя,
//! но не доказывает использование ИИ и не определяет авторство.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Сколько слов в образцах нужно, чтобы профиль считался пригодным.
pub const MIN_PROFILE_WORDS: usize = 300;
/// Минимум слов в проверяемом тексте.
pub const MIN_CHECKED_WORDS: usize = 60;
const MAX_EXAMPLE_CHARS: usize = 160;
const MAX_FRAGMENT_CHARS: usize = 220;

/// Признаки одного текста или профиля. Хранит только числа, короткие списки и
/// несколько коротких примеров — не копию текста.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleFeatures {
    pub words: usize,
    pub sentences: usize,
    pub paragraphs: usize,
    pub sentence_length_mean: f64,
    pub sentence_length_median: f64,
    pub sentence_length_p90: f64,
    pub paragraph_length_mean: f64,
    /// Доля формальных маркеров среди всех маркеров регистра (0..1).
    pub formality: f64,
    /// Вводные конструкции на 100 предложений.
    pub introductory_per_100: f64,
    /// Категоричные маркеры на 100 предложений.
    pub categorical_per_100: f64,
    /// Смягчающие маркеры на 100 предложений.
    pub hedging_per_100: f64,
    pub commas_per_sentence: f64,
    pub dashes_per_100_words: f64,
    pub colons_per_100_words: f64,
    pub semicolons_per_100_words: f64,
    /// «Ёлочки» среди всех кавычек (0..1).
    pub guillemet_share: f64,
    /// Доля строк-пунктов списка среди непустых строк (0..1).
    pub list_line_share: f64,
    /// Заголовки Markdown на 100 строк.
    pub headings_per_100_lines: f64,
    pub connectives: Vec<String>,
    pub frequent_words: Vec<String>,
    pub sentence_openers: Vec<String>,
    pub paragraph_endings: Vec<String>,
    /// Нехарактерные клише, найденные в тексте (для профиля — те, что пользователь всё же использует).
    pub cliches_used: Vec<String>,
    /// Подтверждённые короткие примеры (только у источников профиля).
    pub examples: Vec<String>,
}

const FORMAL_RU: &[&str] = &[
    "следовательно",
    "таким образом",
    "данный",
    "данная",
    "данное",
    "является",
    "являются",
    "осуществляется",
    "в соответствии",
    "необходимо",
    "рассмотрим",
    "согласно",
];
const INFORMAL_RU: &[&str] = &[
    "типа",
    "короче",
    "ну",
    "вообще",
    "прям",
    "кстати",
    "просто",
    "как бы",
    "в общем",
    "вот",
];
const FORMAL_EN: &[&str] = &[
    "therefore",
    "thus",
    "hence",
    "consequently",
    "furthermore",
    "moreover",
    "is defined",
];
const INFORMAL_EN: &[&str] = &[
    "kind of",
    "sort of",
    "basically",
    "actually",
    "pretty",
    "gonna",
    "stuff",
];
const INTRO_RU: &[&str] = &[
    "во-первых",
    "во-вторых",
    "например",
    "итак",
    "однако",
    "кроме того",
    "таким образом",
    "следовательно",
    "в итоге",
    "конечно",
    "наоборот",
    "впрочем",
    "значит",
];
const INTRO_EN: &[&str] = &[
    "first",
    "for example",
    "however",
    "moreover",
    "therefore",
    "in addition",
    "finally",
    "thus",
];
const CATEGORICAL_RU: &[&str] = &[
    "очевидно",
    "безусловно",
    "всегда",
    "никогда",
    "обязательно",
    "точно",
    "несомненно",
    "абсолютно",
];
const CATEGORICAL_EN: &[&str] = &[
    "obviously",
    "clearly",
    "always",
    "never",
    "certainly",
    "undoubtedly",
    "definitely",
];
const HEDGES_RU: &[&str] = &[
    "возможно",
    "вероятно",
    "по-видимому",
    "кажется",
    "скорее",
    "примерно",
    "наверное",
];
const HEDGES_EN: &[&str] = &[
    "perhaps",
    "probably",
    "likely",
    "seems",
    "maybe",
    "approximately",
];
const CONNECTIVES_RU: &[&str] = &[
    "поэтому",
    "потому что",
    "так как",
    "поскольку",
    "однако",
    "но",
    "а также",
    "при этом",
    "тогда",
    "значит",
    "то есть",
    "из-за",
    "в результате",
    "следовательно",
    "таким образом",
];
const CONNECTIVES_EN: &[&str] = &[
    "because",
    "so",
    "but",
    "however",
    "therefore",
    "then",
    "that is",
    "as a result",
    "while",
];
/// Шаблонные обороты, нехарактерные для многих студенческих работ. Их наличие —
/// повод перечитать фрагмент, а не доказательство чего-либо.
const CLICHES_RU: &[&str] = &[
    "важно отметить",
    "стоит отметить",
    "следует отметить",
    "в современном мире",
    "играет ключевую роль",
    "играет важную роль",
    "давайте рассмотрим",
    "в заключение можно сказать",
    "подводя итог",
    "неотъемлемой частью",
    "широкий спектр",
    "является ключевым",
];
const CLICHES_EN: &[&str] = &[
    "it is important to note",
    "in today's world",
    "plays a crucial role",
    "delve into",
    "in conclusion",
    "a wide range of",
    "it is worth noting",
];
const STOP_RU: &[&str] = &[
    "и",
    "в",
    "во",
    "не",
    "что",
    "он",
    "на",
    "я",
    "с",
    "со",
    "как",
    "а",
    "то",
    "все",
    "она",
    "так",
    "его",
    "но",
    "да",
    "ты",
    "к",
    "у",
    "же",
    "вы",
    "за",
    "бы",
    "по",
    "только",
    "ее",
    "её",
    "мне",
    "было",
    "вот",
    "от",
    "меня",
    "еще",
    "ещё",
    "нет",
    "о",
    "из",
    "ему",
    "это",
    "этот",
    "эта",
    "для",
    "при",
    "или",
    "если",
    "то",
    "мы",
    "их",
    "они",
    "также",
    "есть",
    "был",
    "быть",
    "по",
    "до",
];
const STOP_EN: &[&str] = &[
    "the", "a", "an", "and", "or", "of", "to", "in", "is", "are", "was", "it", "that", "this",
    "for", "on", "with", "as", "be", "by", "at", "we", "i", "not",
];

pub fn lists(language: &str) -> Lists {
    if language == "en" {
        Lists {
            formal: FORMAL_EN,
            informal: INFORMAL_EN,
            intro: INTRO_EN,
            categorical: CATEGORICAL_EN,
            hedges: HEDGES_EN,
            connectives: CONNECTIVES_EN,
            cliches: CLICHES_EN,
            stop: STOP_EN,
        }
    } else {
        Lists {
            formal: FORMAL_RU,
            informal: INFORMAL_RU,
            intro: INTRO_RU,
            categorical: CATEGORICAL_RU,
            hedges: HEDGES_RU,
            connectives: CONNECTIVES_RU,
            cliches: CLICHES_RU,
            stop: STOP_RU,
        }
    }
}

pub struct Lists {
    pub formal: &'static [&'static str],
    pub informal: &'static [&'static str],
    pub intro: &'static [&'static str],
    pub categorical: &'static [&'static str],
    pub hedges: &'static [&'static str],
    pub connectives: &'static [&'static str],
    pub cliches: &'static [&'static str],
    pub stop: &'static [&'static str],
}

pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '\''))
        .map(|w| w.trim_matches('-').to_lowercase())
        .filter(|w| w.chars().any(char::is_alphabetic))
        .collect()
}

/// Предложения без строк-заголовков и с сохранением исходного текста.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("```") {
            continue;
        }
        let line = strip_list_marker(line);
        let mut current = String::new();
        let chars: Vec<char> = line.chars().collect();
        for (index, ch) in chars.iter().enumerate() {
            current.push(*ch);
            let end = matches!(ch, '.' | '!' | '?' | '…');
            let next_space = chars.get(index + 1).is_none_or(|n| n.is_whitespace());
            if end && next_space {
                push_sentence(&mut out, &current);
                current.clear();
            }
        }
        push_sentence(&mut out, &current);
    }
    out
}

fn push_sentence(out: &mut Vec<String>, raw: &str) {
    let sentence = raw.trim();
    if words(sentence).len() >= 2 {
        out.push(sentence.to_owned());
    }
}

fn strip_list_marker(line: &str) -> &str {
    let trimmed = line.trim_start_matches(['-', '*', '+', '•']).trim_start();
    if trimmed.len() != line.len() {
        return trimmed;
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && line[digits..].starts_with(['.', ')']) {
        return line[digits + 1..].trim_start();
    }
    line
}

fn is_list_line(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("- ") || line.starts_with("* ") || line.starts_with("• ") || {
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && (line[digits..].starts_with(". ") || line[digits..].starts_with(") "))
    }
}

pub fn count_phrase(haystack_lower: &str, phrase: &str) -> usize {
    // Совпадение по границам слов.
    let mut count = 0;
    let mut start = 0;
    while let Some(found) = haystack_lower[start..].find(phrase) {
        let at = start + found;
        let before = haystack_lower[..at].chars().next_back();
        let after = haystack_lower[at + phrase.len()..].chars().next();
        if before.is_none_or(|c| !c.is_alphanumeric()) && after.is_none_or(|c| !c.is_alphanumeric())
        {
            count += 1;
        }
        start = at + phrase.len();
    }
    count
}

fn per(count: usize, base: usize, scale: f64) -> f64 {
    if base == 0 {
        0.0
    } else {
        count as f64 * scale / base as f64
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn quantile(sorted: &[usize], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[index] as f64
}

fn top(counts: HashMap<String, usize>, limit: usize, min: usize) -> Vec<String> {
    let mut items: Vec<_> = counts.into_iter().filter(|(_, n)| *n >= min).collect();
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    items.into_iter().take(limit).map(|(w, _)| w).collect()
}

pub fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_owned()
    } else {
        let mut out: String = text.chars().take(limit.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Признаки текста. `with_examples` — сохранить до 3 коротких предложений-образцов.
pub fn analyze(text: &str, language: &str, with_examples: bool) -> StyleFeatures {
    let l = lists(language);
    let lower = text.to_lowercase();
    let all_words = words(text);
    let sentences = sentences(text);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let paragraphs: Vec<&str> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty() && !p.starts_with('#'))
        .collect();
    let mut lengths: Vec<usize> = sentences.iter().map(|s| words(s).len()).collect();
    lengths.sort_unstable();
    let sentence_count = sentences.len();
    let mean = per(lengths.iter().sum(), sentence_count, 1.0);
    let paragraph_sentences: usize = paragraphs.iter().map(|p| self::sentences(p).len()).sum();

    let sum = |phrases: &[&str]| {
        phrases
            .iter()
            .map(|p| count_phrase(&lower, p))
            .sum::<usize>()
    };
    let formal = sum(l.formal);
    let informal = sum(l.informal);
    let quotes_guillemet = text.matches('«').count();
    let quotes_plain = text.matches('"').count() / 2 + text.matches('“').count();

    let mut connective_counts = HashMap::new();
    for connective in l.connectives {
        let n = count_phrase(&lower, connective);
        if n > 0 {
            connective_counts.insert((*connective).to_owned(), n);
        }
    }
    let mut word_counts: HashMap<String, usize> = HashMap::new();
    for word in &all_words {
        if word.chars().count() > 3 && !l.stop.contains(&word.as_str()) {
            *word_counts.entry(word.clone()).or_default() += 1;
        }
    }
    let mut openers: HashMap<String, usize> = HashMap::new();
    for sentence in &sentences {
        if let Some(first) = words(sentence).into_iter().next() {
            *openers.entry(first).or_default() += 1;
        }
    }
    let mut endings: HashMap<String, usize> = HashMap::new();
    for paragraph in &paragraphs {
        if let Some(last) = self::sentences(paragraph).last() {
            let tail: Vec<String> = words(last).into_iter().rev().take(2).collect();
            if !tail.is_empty() {
                let mut tail = tail;
                tail.reverse();
                *endings.entry(tail.join(" ")).or_default() += 1;
            }
        }
    }
    let examples = if with_examples {
        sentences
            .iter()
            .filter(|s| {
                let n = words(s).len();
                (6..=24).contains(&n) && s.chars().count() <= MAX_EXAMPLE_CHARS
            })
            .take(3)
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    StyleFeatures {
        words: all_words.len(),
        sentences: sentence_count,
        paragraphs: paragraphs.len(),
        sentence_length_mean: round2(mean),
        sentence_length_median: quantile(&lengths, 0.5),
        sentence_length_p90: quantile(&lengths, 0.9),
        paragraph_length_mean: round2(per(paragraph_sentences, paragraphs.len(), 1.0)),
        formality: round2(if formal + informal == 0 {
            0.5
        } else {
            formal as f64 / (formal + informal) as f64
        }),
        introductory_per_100: round2(per(sum(l.intro), sentence_count, 100.0)),
        categorical_per_100: round2(per(sum(l.categorical), sentence_count, 100.0)),
        hedging_per_100: round2(per(sum(l.hedges), sentence_count, 100.0)),
        commas_per_sentence: round2(per(text.matches(',').count(), sentence_count, 1.0)),
        dashes_per_100_words: round2(per(
            text.matches(" — ").count() + text.matches(" - ").count() + text.matches(" – ").count(),
            all_words.len(),
            100.0,
        )),
        colons_per_100_words: round2(per(text.matches(':').count(), all_words.len(), 100.0)),
        semicolons_per_100_words: round2(per(text.matches(';').count(), all_words.len(), 100.0)),
        guillemet_share: round2(if quotes_guillemet + quotes_plain == 0 {
            0.5
        } else {
            quotes_guillemet as f64 / (quotes_guillemet + quotes_plain) as f64
        }),
        list_line_share: round2(per(
            lines.iter().filter(|line| is_list_line(line)).count(),
            lines.len(),
            1.0,
        )),
        headings_per_100_lines: round2(per(
            lines
                .iter()
                .filter(|line| line.trim_start().starts_with('#'))
                .count(),
            lines.len(),
            100.0,
        )),
        connectives: top(connective_counts, 8, 1),
        frequent_words: top(word_counts, 15, 2),
        sentence_openers: top(openers, 8, 2),
        paragraph_endings: top(endings, 5, 2),
        cliches_used: l
            .cliches
            .iter()
            .filter(|c| count_phrase(&lower, c) > 0)
            .map(|c| (*c).to_owned())
            .collect(),
        examples,
    }
}

/// Объединение признаков нескольких источников: средние, взвешенные по числу
/// предложений; списки — объединение с сохранением порядка.
pub fn merge(parts: &[StyleFeatures]) -> Option<StyleFeatures> {
    let total_sentences: usize = parts.iter().map(|p| p.sentences).sum();
    if parts.is_empty() || total_sentences == 0 {
        return None;
    }
    let weighted = |get: &dyn Fn(&StyleFeatures) -> f64| {
        round2(
            parts
                .iter()
                .map(|p| get(p) * p.sentences as f64)
                .sum::<f64>()
                / total_sentences as f64,
        )
    };
    let union = |get: &dyn Fn(&StyleFeatures) -> &Vec<String>, limit: usize| {
        let mut out: Vec<String> = Vec::new();
        for part in parts {
            for item in get(part) {
                if !out.contains(item) && out.len() < limit {
                    out.push(item.clone());
                }
            }
        }
        out
    };
    Some(StyleFeatures {
        words: parts.iter().map(|p| p.words).sum(),
        sentences: total_sentences,
        paragraphs: parts.iter().map(|p| p.paragraphs).sum(),
        sentence_length_mean: weighted(&|p| p.sentence_length_mean),
        sentence_length_median: weighted(&|p| p.sentence_length_median),
        sentence_length_p90: weighted(&|p| p.sentence_length_p90),
        paragraph_length_mean: weighted(&|p| p.paragraph_length_mean),
        formality: weighted(&|p| p.formality),
        introductory_per_100: weighted(&|p| p.introductory_per_100),
        categorical_per_100: weighted(&|p| p.categorical_per_100),
        hedging_per_100: weighted(&|p| p.hedging_per_100),
        commas_per_sentence: weighted(&|p| p.commas_per_sentence),
        dashes_per_100_words: weighted(&|p| p.dashes_per_100_words),
        colons_per_100_words: weighted(&|p| p.colons_per_100_words),
        semicolons_per_100_words: weighted(&|p| p.semicolons_per_100_words),
        guillemet_share: weighted(&|p| p.guillemet_share),
        list_line_share: weighted(&|p| p.list_line_share),
        headings_per_100_lines: weighted(&|p| p.headings_per_100_lines),
        connectives: union(&|p| &p.connectives, 12),
        frequent_words: union(&|p| &p.frequent_words, 25),
        sentence_openers: union(&|p| &p.sentence_openers, 12),
        paragraph_endings: union(&|p| &p.paragraph_endings, 8),
        cliches_used: union(&|p| &p.cliches_used, 20),
        examples: union(&|p| &p.examples, 9),
    })
}

/// Различие одного признака: что в профиле, что в тексте и почему это важно.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureDelta {
    pub feature: String,
    pub label: String,
    pub profile: f64,
    pub checked: f64,
    /// Вклад в оценку расхождения (0..1).
    pub weight: f64,
}

/// Нехарактерный фрагмент с объяснением.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleFragment {
    pub text: String,
    pub features: Vec<String>,
    pub explanation: String,
}

pub struct Comparison {
    /// Эвристическое расхождение 0..10 (0 — близко к профилю).
    pub divergence: u8,
    pub deltas: Vec<FeatureDelta>,
    pub fragments: Vec<StyleFragment>,
}

fn relative(profile: f64, checked: f64, floor: f64) -> f64 {
    ((checked - profile).abs() / profile.abs().max(floor)).min(1.0)
}

/// Сравнение текста с профилем.
pub fn compare(profile: &StyleFeatures, text: &str, language: &str) -> Comparison {
    let checked = analyze(text, language, false);
    let candidates = [
        (
            "sentence_length_mean",
            "Средняя длина предложения (слов)",
            profile.sentence_length_mean,
            checked.sentence_length_mean,
            4.0,
            2.0,
        ),
        (
            "commas_per_sentence",
            "Запятых на предложение",
            profile.commas_per_sentence,
            checked.commas_per_sentence,
            0.5,
            1.0,
        ),
        (
            "formality",
            "Формальность (доля формальных маркеров)",
            profile.formality,
            checked.formality,
            0.3,
            1.5,
        ),
        (
            "introductory_per_100",
            "Вводные конструкции на 100 предложений",
            profile.introductory_per_100,
            checked.introductory_per_100,
            10.0,
            1.0,
        ),
        (
            "categorical_per_100",
            "Категоричные утверждения на 100 предложений",
            profile.categorical_per_100,
            checked.categorical_per_100,
            5.0,
            1.0,
        ),
        (
            "dashes_per_100_words",
            "Тире на 100 слов",
            profile.dashes_per_100_words,
            checked.dashes_per_100_words,
            0.5,
            0.7,
        ),
        (
            "list_line_share",
            "Доля строк-списков",
            profile.list_line_share,
            checked.list_line_share,
            0.15,
            0.8,
        ),
        (
            "paragraph_length_mean",
            "Предложений в абзаце",
            profile.paragraph_length_mean,
            checked.paragraph_length_mean,
            2.0,
            0.7,
        ),
    ];
    let total_weight: f64 = candidates.iter().map(|c| c.5).sum();
    let mut score = 0.0;
    let mut deltas = Vec::new();
    for (feature, label, p, c, floor, weight) in candidates {
        let diff = relative(p, c, floor);
        score += diff * weight;
        if diff >= 0.35 {
            deltas.push(FeatureDelta {
                feature: feature.to_owned(),
                label: label.to_owned(),
                profile: p,
                checked: c,
                weight: round2(diff),
            });
        }
    }
    let new_cliches: Vec<&String> = checked
        .cliches_used
        .iter()
        .filter(|c| !profile.cliches_used.contains(c))
        .collect();
    let mut divergence = score / total_weight * 10.0 + (new_cliches.len() as f64 * 0.5).min(2.0);
    if divergence > 10.0 {
        divergence = 10.0;
    }
    deltas.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let l = lists(language);
    let long_limit = (profile.sentence_length_p90 * 1.6)
        .max(profile.sentence_length_mean * 2.2)
        .max(18.0);
    let mut fragments = Vec::new();
    for sentence in sentences(text) {
        if fragments.len() >= 10 {
            break;
        }
        let lower = sentence.to_lowercase();
        let mut features = Vec::new();
        let mut reasons = Vec::new();
        for cliche in &new_cliches {
            if count_phrase(&lower, cliche) > 0 {
                features.push("cliche".to_owned());
                reasons.push(format!(
                    "оборот «{cliche}» не встречается в подтверждённых работах"
                ));
            }
        }
        let length = words(&sentence).len() as f64;
        if length > long_limit {
            features.push("sentence_length".to_owned());
            reasons.push(format!(
                "{length:.0} слов — длиннее, чем обычно в ваших работах (обычно до {:.0})",
                profile.sentence_length_p90
            ));
        }
        if profile.categorical_per_100 < 5.0
            && l.categorical.iter().any(|m| count_phrase(&lower, m) > 0)
        {
            features.push("categorical".to_owned());
            reasons.push("категоричная формулировка, а в ваших работах они редки".to_owned());
        }
        if !reasons.is_empty() {
            fragments.push(StyleFragment {
                text: truncate_chars(&sentence, MAX_FRAGMENT_CHARS),
                features,
                explanation: format!(
                    "{}. Это повод перечитать фрагмент, а не доказательство.",
                    capitalize(&reasons.join("; "))
                ),
            });
        }
    }
    Comparison {
        divergence: divergence.round().clamp(0.0, 10.0) as u8,
        deltas,
        fragments,
    }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Явные правила из Markdown-описания стиля: пункты списков, коротко.
pub fn declared_rules(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .filter(|line| is_list_line(line))
        .map(|line| truncate_chars(strip_list_marker(line.trim()), 200))
        .filter(|line| !line.is_empty())
        .take(20)
        .collect()
}
