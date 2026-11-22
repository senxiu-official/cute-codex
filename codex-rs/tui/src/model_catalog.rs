//! TUI model and collaboration inventories; refreshing models preserves the server mode catalog.

use crate::backend_id::BackendId;
use codex_protocol::config_types::CollaborationModeMask;
use codex_protocol::openai_models::ModelPreset;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Held during the config RPC. A confirmed or possible switch makes this backend's untagged model list
/// unsafe to apply for the rest of this TUI session, including after reconnecting.
pub(crate) type ProviderWrite = Arc<Mutex<bool>>;

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
    pub(crate) backend: Option<BackendId>,
    pub(crate) provider_write: ProviderWrite,
    pub(crate) models: Vec<ModelPreset>,
    pub(crate) collaboration_modes: Vec<CollaborationModeMask>,
}

impl ModelCatalog {
    pub(crate) fn new(models: Vec<ModelPreset>) -> Self {
        Self {
            backend: None,
            provider_write: ProviderWrite::default(),
            models,
            collaboration_modes: Vec::new(),
        }
    }

    pub(crate) fn for_backend(
        mut self,
        backend: BackendId,
        writes: &mut HashMap<BackendId, ProviderWrite>,
    ) -> Self {
        self.backend = Some(backend);
        self.provider_write = writes.entry(backend).or_default().clone();
        self
    }

    pub(crate) fn provider_may_have_changed(&self) -> bool {
        self.provider_write
            .try_lock()
            .map_or(true, |changed| *changed)
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
