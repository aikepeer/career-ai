//! Candidate Profile & Keyword Manager Handlers for Career-AI Dashboard.

pub mod config_merge;
pub mod keywords;
pub mod pipeline;
pub mod pipeline_args;
pub mod pipeline_types;
pub mod profile;
pub mod sources;
mod util;

pub use config_merge::update_llm_settings_in_config;
pub(crate) use config_merge::{merge_generated_config, merge_generated_config_doc};
pub use keywords::{api_config_keywords, load_keywords_from_config};
pub use pipeline::{api_cli_run, api_pipeline_discover, api_pipeline_match};
pub use profile::{
    api_profile_save, confirm_profile_import, load_profile_view, save_profile_draft,
};
pub use sources::build_sources_list;
