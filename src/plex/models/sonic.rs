//! Sonic similarity models.

use super::Track;
use serde::{Deserialize, Serialize};

/// Sonic similarity data.
#[derive(Debug, Clone)]
pub struct SonicSimilar {
    pub tracks: Vec<Track>,
}

/// Response wrapper for related/similar items.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RelatedResponse {
    pub media_container: RelatedContainer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RelatedContainer {
    #[serde(default, rename = "Hub")]
    pub hub: Vec<RelatedHub>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedHub {
    pub hub_identifier: String,
    pub title: String,
    #[serde(rename = "type")]
    pub hub_type: String,
    #[serde(default)]
    pub size: u32,
    #[serde(default, rename = "Metadata")]
    pub metadata: Vec<serde_json::Value>,
}
