//! `careerai sources sync` — probes the seeded ATS list for companies
//! whose currently-open jobs match the user's `domains:` keywords and
//! either previews the diff or merges it into `config/local.yaml`.
//!
//! Default behavior is preview. `--apply` is required to mutate the file.
//! Split into per-concern submodules to stay under the 300-LOC cap.

mod merge;
mod run;
#[cfg(test)]
mod tests;

pub use run::run;
