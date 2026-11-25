//! Bounded Needleman-Wunsch alignment over CallSig strings.
//!
//! The alignment never collapses the two mutation kinds: a call that exists
//! only on one side is an `Insertion` or a `Deletion`, while two different
//! calls at the same aligned column are a `Substitution`. That distinction is
//! the whole point of distinguishing mutation kinds: "a tool was added" and
//! "the task was done another way" are different observations and are reported
//! as different numbers.
//!
//! The dynamic programme is band-limited: only cells with `|i - j| <= band`
//! are visited, and a pair whose length difference exceeds the band is an
//! error instead of a silent truncation. Tie-breaking is fixed (diagonal,
//! then deletion, then insertion), so the same inputs always produce the same
//! operations.

use ase_core::{err, Result};

const MATCH_SCORE: i32 = 2;
const SUBSTITUTION_SCORE: i32 = -1;
const GAP_SCORE: i32 = -2;
const UNREACHABLE: i32 = i32::MIN / 4;

/// Band for one alignment. A caller parameter, not a measured gate.
#[derive(Clone, Copy, Debug)]
pub struct AlignConfig {
    pub band: u32,
}

impl Default for AlignConfig {
    fn default() -> Self {
        Self { band: 16 }
    }
}

impl AlignConfig {
    pub fn validate(&self) -> Result<()> {
        if self.band == 0 || self.band > 4096 {
            return Err(err("alignment band must be in 1..=4096"));
        }
        Ok(())
    }
}

/// One step of an alignment, in motif order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignOp {
    /// Same signature at both positions.
    Match { motif: u32, other: u32 },
    /// Different signature at the same aligned column.
    Substitution { motif: u32, other: u32 },
    /// A call that only the other sequence has: the sequence inserted it.
    Insertion { other: u32 },
    /// A motif column the other sequence does not have.
    Deletion { motif: u32 },
}

/// Result of aligning one sequence against a motif skeleton.
#[derive(Clone, Debug)]
pub struct Alignment {
    pub ops: Vec<AlignOp>,
    pub matches: u32,
    pub substitutions: u32,
    pub insertions: u32,
    pub deletions: u32,
}

impl Alignment {
    /// Substitutions + insertions + deletions; matches are not a change.
    pub fn changes(&self) -> u32 {
        self.substitutions + self.insertions + self.deletions
    }

    /// Motif columns whose aligned call differs; deterministic ascending order.
    pub fn substituted_columns(&self) -> Vec<u32> {
        let mut columns: Vec<u32> = self
            .ops
            .iter()
            .filter_map(|op| match op {
                AlignOp::Substitution { motif, .. } => Some(*motif),
                _ => None,
            })
            .collect();
        columns.sort_unstable();
        columns
    }
}

/// Align `other` against the motif skeleton, bounded by `config.band`.
pub fn align(motif: &[String], other: &[String], config: &AlignConfig) -> Result<Alignment> {
    config.validate()?;
    if motif.is_empty() || other.is_empty() {
        return Err(err("alignment needs two non-empty sequences"));
    }
    let band = config.band as usize;
    let rows = motif.len();
    let cols = other.len();
    let difference = rows.abs_diff(cols);
    if difference > band {
        return Err(err(format!(
            "sequence length difference {difference} exceeds alignment band {band}"
        )));
    }
    if rows.saturating_mul(cols) > 4_000_000 {
        return Err(err(
            "alignment matrix exceeds the bounded size for this configuration",
        ));
    }

    let width = cols + 1;
    let mut scores = vec![UNREACHABLE; (rows + 1) * width];
    let mut directions = vec![0u8; (rows + 1) * width];
    scores[0] = 0;
    for j in 1..=band.min(cols) {
        scores[j] = GAP_SCORE * j as i32;
        directions[j] = 3;
    }

    for i in 1..=rows {
        let low = i.saturating_sub(band).max(1);
        let high = (i + band).min(cols);
        let row = i * width;
        for j in low..=high {
            let mut best = UNREACHABLE;
            let mut direction = 0u8;

            // Diagonal: match or substitution.
            let diagonal = scores[(i - 1) * width + (j - 1)];
            if diagonal > UNREACHABLE / 2 {
                let score = if motif[i - 1] == other[j - 1] {
                    MATCH_SCORE
                } else {
                    SUBSTITUTION_SCORE
                };
                let candidate = diagonal + score;
                if candidate > best {
                    best = candidate;
                    direction = 1;
                }
            }
            // Up: a motif column the other sequence lacks (deletion).
            let up = scores[(i - 1) * width + j];
            if up > UNREACHABLE / 2 {
                let candidate = up + GAP_SCORE;
                if candidate > best {
                    best = candidate;
                    direction = 2;
                }
            }
            // Left: a call the other sequence added (insertion).
            let left = scores[row + (j - 1)];
            if left > UNREACHABLE / 2 {
                let candidate = left + GAP_SCORE;
                if candidate > best {
                    best = candidate;
                    direction = 3;
                }
            }
            scores[row + j] = best;
            directions[row + j] = direction;
        }
    }

    if scores[rows * width + cols] <= UNREACHABLE / 2 {
        return Err(err(
            "no alignment path exists inside the configured band",
        ));
    }

    let mut ops = Vec::new();
    let mut matches = 0u32;
    let mut substitutions = 0u32;
    let mut insertions = 0u32;
    let mut deletions = 0u32;
    let mut i = rows;
    let mut j = cols;
    while i > 0 || j > 0 {
        match directions[i * width + j] {
            1 => {
                if motif[i - 1] == other[j - 1] {
                    matches += 1;
                    ops.push(AlignOp::Match {
                        motif: (i - 1) as u32,
                        other: (j - 1) as u32,
                    });
                } else {
                    substitutions += 1;
                    ops.push(AlignOp::Substitution {
                        motif: (i - 1) as u32,
                        other: (j - 1) as u32,
                    });
                }
                i -= 1;
                j -= 1;
            }
            2 => {
                deletions += 1;
                ops.push(AlignOp::Deletion { motif: (i - 1) as u32 });
                i -= 1;
            }
            3 => {
                insertions += 1;
                ops.push(AlignOp::Insertion { other: (j - 1) as u32 });
                j -= 1;
            }
            _ => {
                return Err(err(format!(
                    "alignment traceback lost its path at ({i}, {j})"
                )));
            }
        }
    }
    ops.reverse();

    Ok(Alignment {
        ops,
        matches,
        substitutions,
        insertions,
        deletions,
    })
}
