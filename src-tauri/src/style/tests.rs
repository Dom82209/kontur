use super::features::{analyze, compare, declared_rules, merge, sentences};

pub(crate) const SAMPLE_RU: &str = "Сначала я посчитала среднее по каждому месяцу. Потом сравнила его с медианой, потому что в данных есть выбросы.\n\nВ таблице видно, что продажи в марте выше. Это связано с акцией, поэтому март я учитывала отдельно.\n\nДальше я построила сводную таблицу по регионам. Получилось, что север отстаёт, но разница небольшая.";

pub(crate) fn sample_text(repeat: usize) -> String {
    (0..repeat)
        .map(|_| SAMPLE_RU)
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[test]
fn analysis_is_explainable_and_keeps_only_short_examples() {
    let text = sample_text(4);
    let features = analyze(&text, "ru", true);
    assert!(features.words > 150);
    assert_eq!(features.sentences, sentences(&text).len());
    assert!(features.sentence_length_mean > 5.0 && features.sentence_length_mean < 12.0);
    assert!(features
        .connectives
        .iter()
        .any(|c| c == "поэтому" || c == "потому что"));
    assert!(features.examples.len() <= 3);
    assert!(features.examples.iter().all(|e| e.chars().count() <= 160));
    // В профиле нет копии текста: сериализованные признаки много меньше исходника.
    let json = serde_json::to_string(&features).unwrap();
    assert!(json.len() < text.len());
}

#[test]
fn compare_explains_uncharacteristic_fragments_without_claiming_ai() {
    let profile = merge(&[analyze(&sample_text(4), "ru", true)]).unwrap();
    let similar = compare(&profile, &sample_text(2), "ru");
    assert!(similar.divergence <= 2, "{}", similar.divergence);
    assert!(similar.fragments.is_empty());

    let odd = "Важно отметить, что в современном мире аналитика играет ключевую роль. \
        Очевидно, что данный подход безусловно является наилучшим, поскольку он обеспечивает широкий спектр возможностей для принятия решений, \
        позволяет учитывать множество факторов одновременно, а также гарантирует высокую точность прогнозов на всех этапах работы организации и её подразделений. \
        Таким образом, следует отметить, что результаты являются несомненно значимыми.";
    let result = compare(&profile, odd, "ru");
    assert!(result.divergence > similar.divergence);
    assert!(!result.fragments.is_empty());
    for fragment in &result.fragments {
        assert!(!fragment.features.is_empty());
        assert!(fragment.explanation.contains("не доказательство"));
        assert!(!fragment.explanation.to_lowercase().contains("нейросет"));
    }
    assert!(result
        .fragments
        .iter()
        .any(|f| f.features.iter().any(|x| x == "cliche")));
    assert!(!result.deltas.is_empty());
}

#[test]
fn declared_rules_come_from_markdown_lists_only() {
    let md = "# Мой стиль\nПишу просто.\n- Без канцелярита\n- Короткие выводы\n1. Формулы в LaTeX";
    assert_eq!(
        declared_rules(md),
        vec!["Без канцелярита", "Короткие выводы", "Формулы в LaTeX"]
    );
}
