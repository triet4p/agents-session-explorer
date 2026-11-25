//! Published bundle, render and local-map schema identifiers.
//!
//! The exporter writes these names and the verifier, importer and join read
//! them back, so both sides share one owner for the on-disk contract. A
//! changed render or policy version therefore fails closed on every route.

pub(crate) const BUNDLE_VERSION: &str = "ase-bundle-v1";
pub(crate) const MOTIF_SCHEMA: &str = "ase-motif-bundle-v1";
pub(crate) const REDACTION_POLICY: &str = "ase-redact-v1";
pub(crate) const EVIDENCE_RENDER: &str = "ase-evidence-md-v1";
pub(crate) const QUESTIONS_RENDER: &str = "ase-questions-md-v1";
pub(crate) const REPRESENTATIVE_POLICY: &str = "ase-representatives-v1";
pub(crate) const SELECTOR_VERSION: &str = "block-v1";
pub(crate) const LOCAL_MAP_SCHEMA: &str = "ase-local-sources-v1";
