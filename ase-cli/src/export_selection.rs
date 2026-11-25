//! Representative selection over prepared occurrences.
//!
//! Earlier and later occurrences anchor the set, then the most divergent and
//! one failure slot join before fill order. Selection is deterministic and
//! bounded; assembly drops the least important slots when the byte ceiling is
//! exceeded, never `earliest` or `latest`.

use crate::export_evidence::PreparedOccurrence;
use crate::export_policy::MAX_REPRESENTATIVES;
use std::collections::HashMap;

fn slot_rank(label: &str) -> u64 {
    match label {
        "earliest" => 0,
        "latest" => 1,
        "most_divergent" => 2,
        "failure" => 3,
        other => other
            .strip_prefix("fill:")
            .and_then(|n| n.parse::<u64>().ok())
            .map(|n| 4 + n)
            .unwrap_or(1000),
    }
}

pub(crate) fn select_representatives(prepared: &[PreparedOccurrence]) -> Vec<(usize, String)> {
    let mut assigned: HashMap<usize, String> = HashMap::new();
    let mut by_key: Vec<usize> = (0..prepared.len()).collect();
    by_key.sort_by_key(|&i| prepared[i].order_key);
    if let Some(&first) = by_key.first() {
        assigned.insert(first, "earliest".to_string());
    }
    if let Some(&last) = by_key.last() {
        assigned.entry(last).or_insert_with(|| "latest".to_string());
    }
    let mut by_divergence: Vec<usize> = (0..prepared.len()).collect();
    by_divergence.sort_by(|&a, &b| {
        prepared[b]
            .divergence
            .cmp(&prepared[a].divergence)
            .then(prepared[b].call_count.cmp(&prepared[a].call_count))
            .then(prepared[a].occurrence_id.cmp(&prepared[b].occurrence_id))
    });
    if let Some(index) = by_divergence.iter().find(|&&i| !assigned.contains_key(&i)).copied() {
        assigned.insert(index, "most_divergent".to_string());
    }
    if let Some(index) = by_key
        .iter()
        .find(|&&i| prepared[i].failure && !assigned.contains_key(&i))
        .copied()
    {
        assigned.insert(index, "failure".to_string());
    }
    let mut fill = 0u64;
    for &index in &by_key {
        if assigned.contains_key(&index) {
            continue;
        }
        fill += 1;
        assigned.insert(index, format!("fill:{fill}"));
    }
    let mut out: Vec<(usize, String)> = assigned.into_iter().collect();
    out.sort_by_key(|(index, label)| (slot_rank(label), prepared[*index].occurrence_id.clone()));
    out
}

pub(crate) fn removable_position(selected: &[(usize, String)]) -> Option<usize> {
    selected
        .iter()
        .enumerate()
        .filter(|(_, entry)| !matches!(entry.1.as_str(), "earliest" | "latest"))
        .map(|(position, entry)| (slot_rank(&entry.1), position))
        .max_by_key(|(rank, position)| (*rank, *position))
        .map(|(_, position)| position)
}

pub(crate) fn cap_selection(selected: &mut Vec<(usize, String)>) {
    while selected.len() > MAX_REPRESENTATIVES {
        let Some(position) = removable_position(selected) else {
            break;
        };
        selected.remove(position);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occurrence(index: u64, failure: bool) -> PreparedOccurrence {
        PreparedOccurrence {
            occurrence_id: format!("occ-{index:016x}"),
            order_key: (0, index, index as u32),
            call_count: 3,
            divergence: 0,
            failure,
            result_status: "absent".to_string(),
            source_alias: "src-test".to_string(),
            evidence: Vec::new(),
            identity: "0".repeat(64),
            private: Vec::new(),
        }
    }

    fn labels(selected: &[(usize, String)]) -> Vec<String> {
        selected.iter().map(|(_, label)| label.clone()).collect()
    }

    #[test]
    fn selection_is_bounded_to_five_before_rendering() {
        let prepared: Vec<PreparedOccurrence> = (0..7).map(|i| occurrence(i, false)).collect();
        let mut selected = select_representatives(&prepared);
        assert_eq!(selected.len(), 7);
        cap_selection(&mut selected);
        assert_eq!(selected.len(), MAX_REPRESENTATIVES);
        let labels = labels(&selected);
        for required in ["earliest", "latest", "most_divergent", "fill:1", "fill:2"] {
            assert!(labels.iter().any(|label| label == required), "missing {required}: {labels:?}");
        }
        assert!(!labels.iter().any(|label| label == "fill:3" || label == "fill:4"));
    }

    #[test]
    fn singleton_keeps_earliest_precedence() {
        let prepared = vec![occurrence(0, false)];
        let mut selected = select_representatives(&prepared);
        cap_selection(&mut selected);
        assert_eq!(labels(&selected), vec!["earliest".to_string()]);
    }

    #[test]
    fn failure_slot_is_replaced_before_earliest_or_latest() {
        let prepared: Vec<PreparedOccurrence> = (0..6)
            .map(|i| occurrence(i, i == 3))
            .collect();
        let mut selected = select_representatives(&prepared);
        cap_selection(&mut selected);
        let labels = labels(&selected);
        assert_eq!(labels.len(), 5);
        assert!(labels.iter().any(|label| label == "failure"), "{labels:?}");
        assert!(labels.iter().any(|label| label == "earliest"), "{labels:?}");
        assert!(labels.iter().any(|label| label == "latest"), "{labels:?}");
    }
}
