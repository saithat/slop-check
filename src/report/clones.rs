//! Groups identical block fingerprints into clone groups.

use super::ScannedFile;
use crate::{
    language::Language,
    lines::LineSpan,
    model::{Candidate, CloneGroup, FileSpan, Fingerprint},
};
use hashbrown::HashMap;
use std::{cmp::Reverse, collections::BTreeMap};

pub(super) struct Clones {
    pub groups: Vec<CloneGroup>,
    /// For each file in scan order, its disjoint spans covered by reported clones.
    pub coverage: Vec<Vec<LineSpan>>,
}

/// One block with a given fingerprint.
struct Occurrence<'a> {
    /// Index of the file in scan order.
    file: usize,
    candidate: &'a Candidate,
}

pub(super) fn group(files: &[ScannedFile]) -> Clones {
    // Equal fingerprints from different grammars are not evidence of a clone.
    let mut buckets = HashMap::<(Language, Fingerprint), Vec<Occurrence<'_>>>::new();
    for (file, scanned) in files.iter().enumerate() {
        for candidate in &scanned.analysis.candidates {
            buckets
                .entry((scanned.language, candidate.fingerprint))
                .or_default()
                .push(Occurrence { file, candidate });
        }
    }
    let mut groups: Vec<_> = buckets
        .into_iter()
        .filter(|(_, occurrences)| occurrences.len() >= 2)
        .collect();
    // Hash maps have no stable order. Larger matches come first, so suppression below
    // prefers the broadest evidence; the rest of the key breaks every tie canonically.
    groups.sort_by_key(|(key, occurrences)| {
        let first = &occurrences[0];
        (
            Reverse(first.candidate.tokens),
            first.file,
            first.candidate.lines.start(),
            *key,
        )
    });

    let mut coverage: Vec<Coverage> = files.iter().map(|_| Coverage::default()).collect();
    let mut result = vec![];
    for (_, occurrences) in groups {
        // A group is redundant only when every occurrence lies inside clones already
        // reported. Keep it if any file gains covered lines.
        let adds_coverage = occurrences
            .iter()
            .any(|occurrence| !coverage[occurrence.file].contains(occurrence.candidate.lines));
        if !adds_coverage {
            continue;
        }
        let tokens = occurrences[0].candidate.tokens;
        let locations = occurrences
            .into_iter()
            .map(|occurrence| {
                coverage[occurrence.file].insert(occurrence.candidate.lines);
                FileSpan {
                    path: files[occurrence.file].path.clone(),
                    lines: occurrence.candidate.lines,
                }
            })
            .collect();
        result.push(CloneGroup { tokens, locations });
    }
    Clones {
        groups: result,
        coverage: coverage.into_iter().map(Coverage::into_spans).collect(),
    }
}

/// Lines covered by accepted clones, as disjoint and non-adjacent spans keyed by start
/// line. Merging on insertion keeps each containment check to one predecessor lookup.
#[derive(Default)]
struct Coverage(BTreeMap<u32, u32>);

impl Coverage {
    fn contains(&self, span: LineSpan) -> bool {
        self.0
            .range(..=span.start())
            .next_back()
            .is_some_and(|(_, end)| *end >= span.end())
    }

    fn insert(&mut self, span: LineSpan) {
        let (mut start, mut end) = (span.start(), span.end());
        if let Some((&left, &right)) = self.0.range(..=start).next_back() {
            if right.saturating_add(1) >= start {
                start = left;
                end = end.max(right);
                self.0.remove(&left);
            }
        }
        while let Some((&left, &right)) = self.0.range(start..).next() {
            if left > end.saturating_add(1) {
                break;
            }
            end = end.max(right);
            self.0.remove(&left);
        }
        self.0.insert(start, end);
    }

    fn into_spans(self) -> Vec<LineSpan> {
        self.0
            .into_iter()
            .map(|(start, end)| LineSpan::new(start, end))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::Analysis, path::RelativePath};
    use std::path::Path;

    #[test]
    fn tied_groups_have_a_stable_fingerprint_order() {
        let candidate = |byte, end| Candidate {
            fingerprint: Fingerprint([byte; 16]),
            lines: LineSpan::new(1, end),
            tokens: 10,
        };
        let file = |path: &str, candidates| ScannedFile {
            path: RelativePath::new(Path::new(path)).unwrap(),
            language: Language::Python,
            analysis: Analysis {
                candidates,
                ..Analysis::default()
            },
        };
        let files = [
            file("a.py", vec![candidate(2, 3), candidate(1, 2)]),
            file("b.py", vec![candidate(1, 2)]),
            file("c.py", vec![candidate(2, 3)]),
        ];

        let groups = group(&files).groups;

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].locations[1].path.as_str(), "b.py");
        assert_eq!(groups[1].locations[1].path.as_str(), "c.py");
    }

    #[test]
    fn coverage_merges_adjacent_spans() {
        let mut coverage = Coverage::default();
        coverage.insert(LineSpan::new(5, 7));
        coverage.insert(LineSpan::new(1, 2));
        coverage.insert(LineSpan::new(3, 4));
        assert!(coverage.contains(LineSpan::new(2, 6)));
        assert!(!coverage.contains(LineSpan::new(6, 8)));
        assert_eq!(coverage.into_spans(), [LineSpan::new(1, 7)]);
    }
}
