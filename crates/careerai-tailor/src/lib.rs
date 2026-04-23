//! Resume tailoring + cover letter drafting.
//!
//! Safety invariant: the tailoring step emits a constrained JSON diff that
//! can only reorder or rewrite existing bullets. It MUST NOT invent
//! experience, titles, dates, or employers. The validator in `diff.rs`
//! (added in M3) rejects anything outside that grammar.
