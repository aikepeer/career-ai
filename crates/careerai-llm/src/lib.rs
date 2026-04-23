//! LLM gateway.
//!
//! Single call site for all LLM traffic. Wraps `rig` (multi-provider),
//! handles retries, on-disk caching, and Anthropic prompt-caching for the
//! master-profile block. Implemented in M3.
