//! Pinned export policy: citation ceilings and the exported question set.
//!
//! The exporter writes these limits into the bundle header and enforces them
//! while preparing evidence, selecting representatives and assembling files,
//! so a bundle cannot cite more than its declared ceiling.

pub(crate) const MAX_REPRESENTATIVES: usize = 5;

pub(crate) const MAX_BUNDLE_BYTES: usize = 32_768;
pub(crate) const MAX_CALL_BYTES: usize = 512;
pub(crate) const MAX_OCCURRENCE_BYTES: usize = 4_096;
pub(crate) const MAX_RESULT_LINES: usize = 3;
pub(crate) const QUESTION_TEXTS: [(&str, &str); 3] = [
    ("Q1", "Is this sequence a fixed ritual or a variable procedure?"),
    ("Q2", "Which calls are essential and which are incidental?"),
    (
        "Q3",
        "Should this repetition be codified as a skill or template under the supplied catalog?",
    ),
];
