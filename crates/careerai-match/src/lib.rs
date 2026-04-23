//! Listing-to-profile matcher.
//!
//! Pipeline: hard filters (location, visa, keywords) → fastembed embeddings
//! → cosine rank → threshold. Implemented in M2.
