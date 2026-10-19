//! What the reversible adapters say about one relay model, from the same
//! metadata resolution the router uses.
use super::*;

/// pi-ai's thinking levels, shared by Pi and DeepSeek Harness.
pub(super) const THINKING_LEVELS: [&str; 7] =
    ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

pub(super) struct ModelFacts {
    pub label: String,
    pub projection: super::super::model_policy::ClientProjection,
    /// Declared effort spellings in declaration order; empty when the
    /// deployment declares no effort control.
    pub efforts: Vec<String>,
    /// The effort the deployment starts on when it declares one.
    pub default_effort: Option<String>,
    /// Whether the model reasons. Explicit effort levels are also a
    /// capability declaration, so a manually configured, uncatalogued model
    /// with levels stays usable.
    pub reasoning: Option<bool>,
    /// Whether the matched catalog entry takes images; unknown is `None`.
    pub image_input: Option<bool>,
    /// USD prices per million tokens (input, output, cache read, cache
    /// write), only when the catalog price is complete.
    pub price: Option<[f64; 4]>,
}

impl ModelFacts {
    /// The declared effort a pi-ai thinking level sends, `none` standing in for `off`.
    pub fn effort_for(&self, level: &str) -> Option<&str> {
        self.efforts
            .iter()
            .find(|effort| *effort == level || (level == "off" && *effort == "none"))
            .map(String::as_str)
    }

    /// The pi-ai thinking levels this deployment offers, in pi-ai's order.
    pub fn thinking_levels(&self) -> Vec<&'static str> {
        if self.reasoning == Some(false) {
            return Vec::new();
        }
        THINKING_LEVELS
            .into_iter()
            .filter(|level| self.effort_for(level).is_some())
            .collect()
    }

    /// The thinking level the deployment's default effort sends, when offered.
    pub fn default_level(&self) -> Option<&'static str> {
        let effort = self.default_effort.as_deref()?;
        self.thinking_levels().into_iter().find(|level| {
            self.effort_for(level)
                .is_some_and(|declared| declared.eq_ignore_ascii_case(effort))
        })
    }
}

pub(super) fn facts(
    store: &Store,
    provider: &Provider,
    id: &str,
    catalog: &models_dev::Catalog,
    official_models: &[Value],
) -> ModelFacts {
    let official = super::super::router::matched_official_model(official_models, id)
        .filter(|_| super::super::router::official_endpoint_applies(provider));
    let (metadata, projection) = super::super::router::model_projection(
        provider,
        id,
        official,
        catalog,
        &store.global_compression(),
    );
    let label = provider
        .models
        .iter()
        .find(|m| m.id == id)
        .map(|m| m.display_name.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(&metadata.display_name)
        .to_owned();
    let efforts: Vec<String> = metadata
        .reasoning
        .levels
        .value
        .as_ref()
        .map(|levels| levels.iter().map(|level| level.effort.clone()).collect())
        .unwrap_or_default();
    let reasoning = if efforts.is_empty() {
        metadata.reasoning.supports
    } else {
        Some(true)
    };
    // A context-only Fast/preview match does not advertise vision.
    let image_input = if metadata.steps.iter().any(|step| {
        matches!(
            step.step.as_str(),
            "speedVariantReference" | "previewContextReference"
        )
    }) {
        None
    } else {
        metadata
            .matched
            .as_ref()
            .and_then(|reference| catalog.exact_in(&reference.provider_id, &reference.model_id))
            .and_then(|matched| matched.model.modalities.as_ref())
            .map(|modalities| modalities.input.iter().any(|input| input == "image"))
    };
    let price = metadata.price;
    let price = (price.complete && price.currency == "USD").then(|| {
        [
            price.input.unwrap_or(0.0),
            price.output.unwrap_or(0.0),
            price.cache_read.unwrap_or(0.0),
            price.cache_write.unwrap_or(0.0),
        ]
    });
    ModelFacts {
        label,
        projection,
        efforts,
        default_effort: metadata.default_effort.clone(),
        reasoning,
        image_input,
        price,
    }
}
