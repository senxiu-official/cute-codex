//! TUI model and collaboration inventories; refreshing models preserves the server mode catalog.

use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::openai_models::ModelPreset;
use std::convert::Infallible;

pub(crate) const LUNA_RESERVE_MODEL: &str = "gpt-reserve";
pub(crate) const LUNA_MODEL: &str = "gpt-6-luna";

pub(crate) fn model_display_name(model: &str) -> &str {
    if let Some(display) = crate::custom_status_items::model_display_name(model) {
        display
    } else if model.eq_ignore_ascii_case(LUNA_RESERVE_MODEL) {
        "Luna Reserve"
    } else {
        model
    }
}

fn resolve_model_display_name<'a>(
    model: &'a str,
    custom_alias: Option<&'a str>,
    catalog_display_name: Option<&'a str>,
) -> &'a str {
    custom_alias
        .or(catalog_display_name)
        .unwrap_or_else(|| model_display_name(model))
}

#[derive(Debug, Clone)]
pub(crate) struct ModelCatalog {
    pub(crate) models: Vec<ModelPreset>,
    pub(crate) collaboration_modes: Vec<CollaborationModeMask>,
}

impl ModelCatalog {
    pub(crate) fn new(models: Vec<ModelPreset>) -> Self {
        Self {
            models,
            collaboration_modes: Vec::new(),
        }
    }

    pub(crate) fn with_collaboration_modes(mut self, modes: Vec<CollaborationModeMask>) -> Self {
        self.collaboration_modes = modes;
        self
    }

    pub(crate) fn try_list_models(&self) -> Result<Vec<ModelPreset>, Infallible> {
        Ok(self.models.clone())
    }

    pub(crate) fn display_name<'a>(&'a self, model: &'a str) -> &'a str {
        // An explicitly configured Cutex presentation alias is more specific than the
        // backend catalog's generic display name.
        resolve_model_display_name(
            model,
            crate::custom_status_items::model_display_name(model),
            self.catalog_display_name(model),
        )
    }

    fn catalog_display_name<'a>(&'a self, model: &'a str) -> Option<&'a str> {
        self.models
            .iter()
            .find(|preset| preset.model == model)
            .map(|preset| preset.display_name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_display_alias_overrides_catalog_display_name() {
        let model = "gpt-6-astra";
        assert_eq!(
            resolve_model_display_name(model, Some("astra 6"), Some("GPT-6-Astra")),
            "astra 6"
        );
        assert_eq!(
            resolve_model_display_name(model, None, Some("GPT-6-Astra")),
            "GPT-6-Astra"
        );
        assert_eq!(resolve_model_display_name(model, None, None), model);
        assert_eq!(
            resolve_model_display_name(LUNA_RESERVE_MODEL, None, None),
            "Luna Reserve"
        );
    }
}
