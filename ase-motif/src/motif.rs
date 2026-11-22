//! Alignment-backed motif evidence with task-level support and a null model.
//!
//! The layer sits on top of [`crate::projection`] and [`crate::family`]:
//!
//! 1. Every call of every eligible sequence is reopened from raw bytes through
//!    the versioned selector before it can take part in a motif. A motif never
//!    contains an unverified call.
//! 2. Shingle families group the candidate sequences; each family's lowest
//!    member is the deterministic motif skeleton.
//! 3. Sequences are aligned to that skeleton with
//!    [`crate::align`], which keeps insertions and substitutions apart.
//! 4. Support is counted **per task**, never per attempt or per occurrence, so
//!    repeated attempts of one task are one observation. `support_relaxed(g)`
//!    allows at most `g` skipped calls between motif columns, which makes it
//!    monotonic in `g` by construction.
//! 5. A deterministic seeded permutation null re-orders each sequence's calls
//!    and reports the expected support, a z score and a lift.
//!
//! Only metadata is involved: no prose is read and no model is called.

use ase_core::{err, RawStore, Result};
use ase_index::Index;
use ase_postings::publication::Publication;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

use crate::align::{align, Alignment, AlignConfig};
use crate::family::{families, FamilyConfig};
use crate::projection::{reopen_call, CallEvidence, Sequence, TaskSequences};

/// Identifier of the motif rule recorded on a report.
pub const MOTIF_RULE_NAME: &str = "motif-alignment";
pub const MOTIF_RULE_VERSION: &str = "motif-align-v1";

/// Fixed default seed for the permutation null model. Recorded on every run.
pub const NULL_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

#[derive(Clone, Copy, Debug)]
pub struct MotifConfig {
    pub family: FamilyConfig,
    /// Largest relaxed gap reported. `support_relaxed` has `max_gap + 1` slots.
    pub max_gap: u32,
    /// Band for one pairwise alignment.
    pub band: u32,
    /// Most substitutions still classified as a near miss.
    pub max_substitutions: u32,
    /// Permutations in the null model; `0` disables it.
    pub null_permutations: u32,
    /// Fixed seed of the null model; the same seed reproduces the same run.
    pub null_seed: u64,
    /// Cap on retained occurrences per motif; the overflow is counted.
    pub max_occurrences: u32,
    /// Families with fewer distinct tasks are not reported as motifs.
    pub min_tasks: u32,
}

impl Default for MotifConfig {
    fn default() -> Self {
        Self {
            family: FamilyConfig::default(),
            max_gap: 2,
            band: 16,
            max_substitutions: 2,
            null_permutations: 64,
            null_seed: NULL_SEED,
            max_occurrences: 256,
            min_tasks: 2,
        }
    }
}

impl MotifConfig {
    pub fn validate(&self) -> Result<()> {
        self.family.validate()?;
        if self.max_gap > 32 {
            return Err(err("motif max_gap must be <= 32"));
        }
        if self.band == 0 || self.band > 4096 {
            return Err(err("motif band must be in 1..=4096"));
        }
        if self.max_substitutions > 32 {
            return Err(err("motif max_substitutions must be <= 32"));
        }
        if self.null_permutations > 10_000 {
            return Err(err("motif null_permutations must be <= 10000"));
        }
        if self.max_occurrences == 0 || self.max_occurrences > 100_000 {
            return Err(err("motif max_occurrences must be in 1..=100000"));
        }
        if self.min_tasks == 0 {
            return Err(err("motif min_tasks must be >= 1"));
        }
        Ok(())
    }
}

/// Why a task did not contribute sequences at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskExclusionReason {
    /// The dispatch membership yielded no admitted attempt.
    NoAdmittedAttempt,
    /// The admitted attempts projected no call sequence.
    NoCallSequence,
}

impl TaskExclusionReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoAdmittedAttempt => "no_admitted_attempt",
            Self::NoCallSequence => "no_call_sequence",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskExclusion {
    pub task: String,
    pub reason: TaskExclusionReason,
}

/// One motif column and how the aligned family members treated it.
#[derive(Clone, Debug)]
pub struct Column {
    pub index: u32,
    pub signature: String,
    pub matches: u32,
    /// Alternative signatures seen at this column, with their counts.
    pub substitutions: BTreeMap<String, u32>,
    pub missing: u32,
}

/// One concrete embedding of the motif in a sequence.
#[derive(Clone, Debug)]
pub struct Occurrence {
    pub task: String,
    pub attempt: String,
    pub dispatch_id: String,
    pub sequence: u32,
    /// Number of calls skipped between the first and last motif column.
    pub gap: u32,
    pub calls: Vec<OccurrenceCall>,
}

/// One matched motif column with its reopenable evidence.
#[derive(Clone, Debug)]
pub struct OccurrenceCall {
    pub position: u32,
    pub signature: String,
    pub evidence: CallEvidence,
}

/// A sequence that did not contain the motif, with its alignment distances.
#[derive(Clone, Debug)]
pub struct Variant {
    pub task: String,
    pub attempt: String,
    pub sequence: u32,
    pub matches: u32,
    pub substitutions: u32,
    pub insertions: u32,
    pub deletions: u32,
    pub substituted_columns: Vec<u32>,
    /// Substitutions within the configured bound and no missing column.
    pub near_miss: bool,
}

/// Why a sequence in the family could not be aligned.
#[derive(Clone, Debug)]
pub struct SequenceExclusion {
    pub task: String,
    pub attempt: String,
    pub sequence: u32,
    pub reason: String,
}

/// Denominator for one motif: who was measured and who was left out.
#[derive(Clone, Debug)]
pub struct Denominator {
    pub tasks_total: u32,
    pub tasks_in_family: u32,
    pub tasks_with_occurrence: u32,
    pub sequences_in_family: u32,
    pub sequences_with_occurrence: u32,
    pub excluded: Vec<DenominatorExclusion>,
}

#[derive(Clone, Debug)]
pub struct DenominatorExclusion {
    pub task: String,
    pub reason: String,
}

/// Deterministic permutation null over call order.
#[derive(Clone, Debug)]
pub struct NullModel {
    pub permutations: u32,
    pub seed: u64,
    pub observed: u32,
    pub expected: f64,
    pub stddev: f64,
    pub z: Option<f64>,
    pub lift: Option<f64>,
}

/// One motif candidate with its support, evidence and null model.
#[derive(Clone, Debug)]
pub struct Motif {
    pub id: String,
    pub family_id: String,
    pub reference_task: String,
    pub reference_attempt: String,
    pub reference_sequence: u32,
    pub signatures: Vec<String>,
    pub columns: Vec<Column>,
    pub support_exact: u32,
    /// `support_relaxed[g]` allows at most `g` skipped calls; non-decreasing.
    pub support_relaxed: Vec<u32>,
    pub denominator: Denominator,
    pub occurrences: Vec<Occurrence>,
    pub occurrences_truncated: u32,
    pub variants: Vec<Variant>,
    pub sequence_exclusions: Vec<SequenceExclusion>,
    pub null: NullModel,
}

impl Motif {
    /// Distinct tasks whose minimal gap is within `gap`.
    pub fn support_at(&self, gap: u32) -> u32 {
        self.support_relaxed
            .get(gap as usize)
            .copied()
            .unwrap_or_else(|| self.support_relaxed.last().copied().unwrap_or(0))
    }

    pub fn near_misses(&self) -> impl Iterator<Item = &Variant> {
        self.variants.iter().filter(|variant| variant.near_miss)
    }
}

#[derive(Clone, Debug)]
pub struct MotifReport {
    pub rule_name: &'static str,
    pub rule_version: &'static str,
    pub config: MotifConfig,
    pub tasks: u32,
    pub sequences: u32,
    pub families: usize,
    pub families_below_min_tasks: usize,
    pub excluded_tasks: Vec<TaskExclusion>,
    pub motifs: Vec<Motif>,
}

struct Obs<'a> {
    task: String,
    attempt: String,
    dispatch_id: String,
    sequence: &'a Sequence,
    signatures: Vec<String>,
}

struct Collected<'a> {
    observations: Vec<Obs<'a>>,
    tasks: BTreeSet<String>,
    exclusions: Vec<TaskExclusion>,
}

fn collect<'a>(
    published: &Publication,
    idx: &Index,
    store: &dyn RawStore,
    tasks: &'a [TaskSequences],
) -> Result<Collected<'a>> {
    let mut all_tasks: BTreeSet<String> = BTreeSet::new();
    let mut reasons: BTreeMap<String, TaskExclusionReason> = BTreeMap::new();
    let mut per_task: BTreeMap<String, Vec<(&'a crate::membership::AttemptMembership, &'a Sequence)>> =
        BTreeMap::new();

    for task in tasks {
        let key = task
            .task_key
            .clone()
            .unwrap_or_else(|| format!("token:{}", task.token));
        all_tasks.insert(key.clone());
        if task.attempts.is_empty() {
            reasons
                .entry(key)
                .or_insert(TaskExclusionReason::NoAdmittedAttempt);
            continue;
        }
        let entry = per_task.entry(key.clone()).or_default();
        for attempt in &task.attempts {
            for sequence in &attempt.sequences {
                entry.push((&attempt.membership, sequence));
            }
        }
        if entry.is_empty() {
            reasons.insert(key, TaskExclusionReason::NoCallSequence);
        } else {
            reasons.remove(&key);
        }
    }

    for (key, entries) in &per_task {
        if entries.is_empty() {
            reasons
                .entry(key.clone())
                .or_insert(TaskExclusionReason::NoCallSequence);
        } else {
            reasons.remove(key);
        }
    }

    let exclusions: Vec<TaskExclusion> = reasons
        .into_iter()
        .map(|(task, reason)| TaskExclusion { task, reason })
        .collect();

    let mut observations = Vec::new();
    for (task, entries) in per_task {
        for (attempt, sequence) in entries {
            let mut signatures = Vec::with_capacity(sequence.calls.len());
            for call in &sequence.calls {
                signatures.push(reopen_call(published, idx, store, call)?);
            }
            observations.push(Obs {
                task: task.clone(),
                attempt: attempt.attempt.clone(),
                dispatch_id: attempt.dispatch_id.clone(),
                sequence,
                signatures,
            });
        }
    }

    Ok(Collected {
        observations,
        tasks: all_tasks,
        exclusions,
    })
}

/// Greedy earliest embedding of the motif columns inside one sequence.
///
/// Returns the matched call positions and the number of calls skipped between
/// the first and last matched column. Leading and trailing calls are outside
/// the motif and do not count as gaps. The greedy walk minimises that internal
/// span, so a gap of `0` means the motif appears contiguously.
fn gap_embedding(motif: &[String], other: &[String]) -> Option<(Vec<usize>, u32)> {
    let mut matched: Vec<usize> = Vec::with_capacity(motif.len());
    let mut cursor = 0usize;
    for signature in motif {
        let found = other.get(cursor..)?.iter().position(|item| item == signature)? + cursor;
        matched.push(found);
        cursor = found + 1;
    }
    let first = *matched.first()?;
    let last = *matched.last()?;
    let span = last - first + 1;
    let gap = (span - motif.len()) as u32;
    Some((matched, gap))
}

/// Fixed-seed SplitMix64; the null model is reproducible without a dependency.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() % bound as u64) as usize
        }
    }
}

fn fisher_yates(items: &mut [String], rng: &mut SplitMix64) {
    if items.len() < 2 {
        return;
    }
    for index in (1..items.len()).rev() {
        let other = rng.below(index + 1);
        items.swap(index, other);
    }
}

fn seed_for(signatures: &[String], base: u64) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for signature in signatures {
        for byte in signature.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= b'\n' as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    base ^ hash
}

fn motif_id(signatures: &[String]) -> String {
    let mut hasher = Sha256::new();
    for signature in signatures {
        hasher.update(signature.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(16);
    for byte in &digest[..8] {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("mot-{hex}")
}

fn mean(values: &[u32]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().map(|value| *value as f64).sum::<f64>() / values.len() as f64
}

fn stddev(values: &[u32], average: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let variance = values
        .iter()
        .map(|value| {
            let delta = *value as f64 - average;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

/// Build motif evidence over a cohort of projected tasks.
///
/// Each `TaskSequences` contributes one counting unit per distinct task key:
/// multiple attempts and multiple occurrences of the same task never raise the
/// support. Tasks with no admitted attempt or no projected call stay in
/// `excluded_tasks` instead of silently disappearing.
pub fn detect(
    published: &Publication,
    idx: &Index,
    store: &dyn RawStore,
    tasks: &[TaskSequences],
    config: &MotifConfig,
) -> Result<MotifReport> {
    config.validate()?;
    let collected = collect(published, idx, store, tasks)?;
    let observations = collected.observations;
    let sequences: Vec<&Sequence> = observations.iter().map(|obs| obs.sequence).collect();
    let family_report = families(&sequences, &config.family)?;

    let mut motifs = Vec::new();
    let mut below_min_tasks = 0usize;
    let align_config = AlignConfig {
        band: config.band,
    };

    for family in &family_report.families {
        let family_tasks: BTreeSet<&str> = family
            .members
            .iter()
            .map(|index| observations[*index].task.as_str())
            .collect();
        if family_tasks.len() < config.min_tasks as usize {
            below_min_tasks += 1;
            continue;
        }

        let reference = family.members[0];
        let reference_obs = &observations[reference];
        let signatures = reference_obs.signatures.clone();
        let mut columns: Vec<Column> = signatures
            .iter()
            .enumerate()
            .map(|(index, signature)| Column {
                index: index as u32,
                signature: signature.clone(),
                matches: 0,
                substitutions: BTreeMap::new(),
                missing: 0,
            })
            .collect();

        // Distance of every family member to the skeleton. A band rejection is
        // reported per sequence instead of aborting the whole cohort.
        let mut aligned: Vec<Option<Alignment>> = vec![None; observations.len()];
        let mut sequence_exclusions = Vec::new();
        for index in &family.members {
            let obs = &observations[*index];
            match align(&signatures, &obs.signatures, &align_config) {
                Ok(alignment) => {
                    for op in &alignment.ops {
                        match op {
                            crate::align::AlignOp::Match { motif, .. } => {
                                columns[*motif as usize].matches += 1;
                            }
                            crate::align::AlignOp::Substitution { motif, other } => {
                                let alternative = &obs.signatures[*other as usize];
                                *columns[*motif as usize]
                                    .substitutions
                                    .entry(alternative.clone())
                                    .or_insert(0) += 1;
                            }
                            crate::align::AlignOp::Deletion { motif } => {
                                columns[*motif as usize].missing += 1;
                            }
                            crate::align::AlignOp::Insertion { .. } => {}
                        }
                    }
                    aligned[*index] = Some(alignment);
                }
                Err(error) => sequence_exclusions.push(SequenceExclusion {
                    task: obs.task.clone(),
                    attempt: obs.attempt.clone(),
                    sequence: *index as u32,
                    reason: error.to_string(),
                }),
            }
        }

        let mut task_gap: BTreeMap<&str, u32> = BTreeMap::new();
        let mut occurrences = Vec::new();
        let mut occurrences_truncated = 0u32;
        let mut sequences_with_occurrence = 0u32;
        let mut variants = Vec::new();

        for index in &family.members {
            let obs = &observations[*index];
            match gap_embedding(&signatures, &obs.signatures) {
                Some((matched, gap)) => {
                    sequences_with_occurrence += 1;
                    task_gap
                        .entry(obs.task.as_str())
                        .and_modify(|current| *current = (*current).min(gap))
                        .or_insert(gap);
                    if occurrences.len() < config.max_occurrences as usize {
                        let calls = matched
                            .iter()
                            .enumerate()
                            .map(|(position, call_index)| OccurrenceCall {
                                position: position as u32,
                                signature: obs.signatures[*call_index].clone(),
                                evidence: obs.sequence.calls[*call_index].clone(),
                            })
                            .collect();
                        occurrences.push(Occurrence {
                            task: obs.task.clone(),
                            attempt: obs.attempt.clone(),
                            dispatch_id: obs.dispatch_id.clone(),
                            sequence: *index as u32,
                            gap,
                            calls,
                        });
                    } else {
                        occurrences_truncated += 1;
                    }
                }
                None => {
                    if let Some(alignment) = &aligned[*index] {
                        let near_miss = alignment.substitutions >= 1
                            && alignment.substitutions <= config.max_substitutions
                            && alignment.deletions == 0;
                        variants.push(Variant {
                            task: obs.task.clone(),
                            attempt: obs.attempt.clone(),
                            sequence: *index as u32,
                            matches: alignment.matches,
                            substitutions: alignment.substitutions,
                            insertions: alignment.insertions,
                            deletions: alignment.deletions,
                            substituted_columns: alignment.substituted_columns(),
                            near_miss,
                        });
                    }
                }
            }
        }

        let support_relaxed: Vec<u32> = (0..=config.max_gap)
            .map(|gap| {
                task_gap
                    .values()
                    .filter(|observed_gap| **observed_gap <= gap)
                    .count() as u32
            })
            .collect();
        let support_exact = support_relaxed.first().copied().unwrap_or(0);

        // Null model: shuffle each sequence's call order with a fixed seed and
        // recount task-deduplicated exact support.
        let mut permutation_counts: Vec<u32> = Vec::new();
        let seed = seed_for(&signatures, config.null_seed);
        if config.null_permutations > 0 {
            let mut rng = SplitMix64::new(seed);
            let mut shuffled: Vec<String> = Vec::new();
            for _ in 0..config.null_permutations {
                let mut permuted_tasks: BTreeSet<&str> = BTreeSet::new();
                for index in &family.members {
                    let obs = &observations[*index];
                    shuffled.clear();
                    shuffled.extend(obs.signatures.iter().cloned());
                    fisher_yates(&mut shuffled, &mut rng);
                    if matches!(gap_embedding(&signatures, &shuffled), Some((_, 0))) {
                        permuted_tasks.insert(obs.task.as_str());
                    }
                }
                permutation_counts.push(permuted_tasks.len() as u32);
            }
        }
        let expected = mean(&permutation_counts);
        let spread = stddev(&permutation_counts, expected);
        let z = if permutation_counts.is_empty() || spread <= 1e-12 {
            None
        } else {
            Some((support_exact as f64 - expected) / spread)
        };
        let lift = if expected > 0.0 {
            Some(support_exact as f64 / expected)
        } else {
            None
        };
        let null = NullModel {
            permutations: config.null_permutations,
            seed,
            observed: support_exact,
            expected,
            stddev: spread,
            z,
            lift,
        };

        let exclusion_reasons: BTreeMap<&str, &'static str> = collected
            .exclusions
            .iter()
            .map(|exclusion| (exclusion.task.as_str(), exclusion.reason.label()))
            .collect();
        let mut excluded = Vec::new();
        for task in &collected.tasks {
            if !family_tasks.contains(task.as_str()) {
                let reason = exclusion_reasons
                    .get(task.as_str())
                    .copied()
                    .unwrap_or("outside_family");
                excluded.push(DenominatorExclusion {
                    task: task.clone(),
                    reason: reason.to_string(),
                });
            }
        }

        motifs.push(Motif {
            id: motif_id(&signatures),
            family_id: family.id.clone(),
            reference_task: reference_obs.task.clone(),
            reference_attempt: reference_obs.attempt.clone(),
            reference_sequence: reference as u32,
            signatures,
            columns,
            support_exact,
            support_relaxed,
            denominator: Denominator {
                tasks_total: collected.tasks.len() as u32,
                tasks_in_family: family_tasks.len() as u32,
                tasks_with_occurrence: task_gap.len() as u32,
                sequences_in_family: family.members.len() as u32,
                sequences_with_occurrence,
                excluded,
            },
            occurrences,
            occurrences_truncated,
            variants,
            sequence_exclusions,
            null,
        });
    }

    motifs.sort_by(|left, right| {
        right
            .support_exact
            .cmp(&left.support_exact)
            .then_with(|| right.support_at(config.max_gap).cmp(&left.support_at(config.max_gap)))
            .then_with(|| left.id.cmp(&right.id))
    });

    Ok(MotifReport {
        rule_name: MOTIF_RULE_NAME,
        rule_version: MOTIF_RULE_VERSION,
        config: *config,
        tasks: collected.tasks.len() as u32,
        sequences: observations.len() as u32,
        families: family_report.families.len(),
        families_below_min_tasks: below_min_tasks,
        excluded_tasks: collected.exclusions,
        motifs,
    })
}
