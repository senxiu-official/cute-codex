use super::*;
use pretty_assertions::assert_eq;

#[test]
fn local_variants_keep_independent_metadata_and_require_exact_selection() {
    let mut first = remote_model("Astra [400K]", "Astra [400K]", 1);
    first.request_model = Some("gpt-6-astra".to_string());
    first.context_window = Some(400_000);
    first.max_context_window = Some(1_000_000);
    first.auto_compact_token_limit = Some(350_000);
    let mut second = first.clone();
    second.slug = "Astra [285K]".to_string();
    second.display_name = second.slug.clone();
    second.context_window = Some(285_000);
    second.auto_compact_token_limit = Some(240_000);
    let candidates = vec![first, second];
    let config = ModelsManagerConfig::default();
    for expected in &candidates {
        let actual = construct_model_info_from_candidates(&expected.slug, &candidates, &config);
        assert_eq!(&actual, expected);
        assert_eq!(actual.request_model(), "gpt-6-astra");
    }
    for unknown in ["Astra [400K] typo", "provider/Astra [400K]"] {
        let actual = construct_model_info_from_candidates(unknown, &candidates, &config);
        assert_eq!(actual.request_model(), unknown);
        assert!(actual.used_fallback_model_metadata);
    }
}
