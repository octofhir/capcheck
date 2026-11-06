//! What a CapabilityStatement says the server does.

use serde::Deserialize;

/// The parts of a CapabilityStatement that can be put to the test.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capability {
    pub fhir_version: Option<String>,
    pub format: Vec<String>,
    pub patch_format: Vec<String>,
    pub rest: Vec<Rest>,
}

/// One `CapabilityStatement.rest`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Rest {
    pub mode: Option<String>,
    pub interaction: Vec<Interaction>,
    pub operation: Vec<Operation>,
    pub resource: Vec<Resource>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Interaction {
    pub code: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Operation {
    pub name: String,
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SearchParam {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

/// One `CapabilityStatement.rest.resource`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Resource {
    #[serde(rename = "type")]
    pub name: String,
    pub interaction: Vec<Interaction>,
    pub search_param: Vec<SearchParam>,
    pub search_include: Vec<String>,
    pub search_rev_include: Vec<String>,
    pub operation: Vec<Operation>,
    pub versioning: Option<String>,
    pub read_history: Option<bool>,
    pub update_create: Option<bool>,
    pub conditional_create: Option<bool>,
    pub conditional_read: Option<String>,
    pub conditional_update: Option<bool>,
    pub conditional_delete: Option<String>,
}

impl Resource {
    /// Whether the statement lists this interaction.
    #[must_use]
    pub fn declares(&self, code: &str) -> bool {
        self.interaction.iter().any(|item| item.code == code)
    }
}

impl Rest {
    #[must_use]
    pub fn declares(&self, code: &str) -> bool {
        self.interaction.iter().any(|item| item.code == code)
    }
}
