//! Source line numbering shared by functions, findings, clones, and coverage.
//!
//! Tree-sitter reports 0-based rows; reports use 1-based lines. The conversion happens
//! only in this module.

use serde::Serialize;
use std::ops::RangeInclusive;
use tree_sitter::Node;

/// An inclusive range of 1-based source lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct LineSpan {
    #[serde(rename = "start_line")]
    start: u32,
    #[serde(rename = "end_line")]
    end: u32,
}

impl LineSpan {
    /// # Panics
    ///
    /// When `start` is zero or after `end`.
    pub fn new(start: u32, end: u32) -> Self {
        assert!(
            1 <= start && start <= end,
            "invalid line span {start}-{end}"
        );
        Self { start, end }
    }

    /// The lines a syntax node occupies. A node that ends at column 0 ends on the
    /// previous line.
    pub fn of(node: Node<'_>) -> Self {
        let start = node.start_position().row;
        let end = node.end_position();
        let end_row = if end.column == 0 {
            end.row.saturating_sub(1)
        } else {
            end.row
        };
        Self {
            start: line(start),
            end: line(end_row.max(start)),
        }
    }

    pub fn start(self) -> u32 {
        self.start
    }

    pub fn end(self) -> u32 {
        self.end
    }

    /// The 0-based rows tree-sitter uses for these lines.
    pub fn rows(self) -> RangeInclusive<usize> {
        row(self.start)..=row(self.end)
    }
}

/// The sorted numbers of the lines that contain code.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CodeLines(Vec<u32>);

impl CodeLines {
    /// Builds the set from one flag per 0-based row.
    pub fn from_rows(rows: &[bool]) -> Self {
        Self(
            rows.iter()
                .enumerate()
                .filter(|(_, code)| **code)
                .map(|(row, _)| line(row))
                .collect(),
        )
    }

    pub fn count(&self) -> u32 {
        u32::try_from(self.0.len()).unwrap_or(u32::MAX)
    }

    pub fn count_in(&self, span: LineSpan) -> u32 {
        let before = self.0.partition_point(|line| *line < span.start);
        let through = self.0.partition_point(|line| *line <= span.end);
        u32::try_from(through - before).unwrap_or(u32::MAX)
    }

    /// Counts the code lines inside any of `spans`, which may overlap. Each line counts once.
    pub fn count_union(&self, spans: &mut [LineSpan]) -> u32 {
        spans.sort_unstable();
        let mut total = 0;
        let mut open: Option<LineSpan> = None;
        for &span in spans.iter() {
            open = match open {
                Some(current) if span.start <= current.end.saturating_add(1) => Some(LineSpan {
                    start: current.start,
                    end: current.end.max(span.end),
                }),
                Some(current) => {
                    total += self.count_in(current);
                    Some(span)
                }
                None => Some(span),
            };
        }
        total + open.map_or(0, |span| self.count_in(span))
    }
}

/// Files are smaller than 4 GiB, so their row numbers fit in a `u32`.
fn line(row: usize) -> u32 {
    u32::try_from(row + 1).unwrap_or(u32::MAX)
}

fn row(line: u32) -> usize {
    line as usize - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_spans_count_code_lines_once() {
        let lines = CodeLines(vec![1, 2, 4, 7, 8, 9]);
        let mut spans = [
            LineSpan::new(9, 9),
            LineSpan::new(2, 8),
            LineSpan::new(1, 4),
        ];
        assert_eq!(lines.count_union(&mut spans), 6);
        assert_eq!(lines.count_union(&mut [LineSpan::new(3, 6)]), 1);
        assert_eq!(lines.count_union(&mut []), 0);
    }

    #[test]
    #[should_panic(expected = "invalid line span")]
    fn spans_cannot_start_at_line_zero() {
        LineSpan::new(0, 1);
    }
}
