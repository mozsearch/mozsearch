pub mod analysis;
pub mod ontology_mapping;

#[cfg(not(target_arch = "wasm32"))]
pub mod history;

#[cfg(not(target_arch = "wasm32"))]
pub mod analysis_manglings;
#[cfg(not(target_arch = "wasm32"))]
pub mod config;
#[cfg(not(target_arch = "wasm32"))]
pub mod coverage;
#[cfg(not(target_arch = "wasm32"))]
pub mod crossref_converter;
#[cfg(not(target_arch = "wasm32"))]
pub mod crossref_lookup;
#[cfg(not(target_arch = "wasm32"))]
pub mod globbing_file_list;
#[cfg(not(target_arch = "wasm32"))]
pub mod identifiers;
#[cfg(not(target_arch = "wasm32"))]
pub mod merger;
#[cfg(not(target_arch = "wasm32"))]
pub mod per_file_info;
#[cfg(not(target_arch = "wasm32"))]
pub mod repo_data_ingestion;
