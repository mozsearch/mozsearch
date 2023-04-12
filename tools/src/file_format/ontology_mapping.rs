use serde::{Deserialize};
use ustr::UstrMap;

#[derive(Deserialize)]
pub struct OntologyMappingConfig {
    #[serde(default)]
    pub pretty: UstrMap<OntologyRule>,
}

#[derive(Deserialize)]
pub struct OntologyRule {
    /// When specified, treats the given symbol/identifier as an nsIRunnable::Run
    /// style method where overrides should be treated as runnables and have
    /// ontology slots allocated to point to the concrete constructors.
    #[serde(default)]
    pub runnable: bool,
}

pub struct OntologyMappingIngestion {
    pub config: OntologyMappingConfig,
}

impl OntologyMappingIngestion {
    pub fn new(config_str: &str) -> Result<Self, String> {
        let config: OntologyMappingConfig =
            toml::from_str(config_str).map_err(|err| err.to_string())?;

        Ok(OntologyMappingIngestion {
            config,
        })
    }
}
