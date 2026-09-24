//! Per-file analysis: one parse and one syntax-tree pass per source file.
//!
//! [`traversal`] walks the tree and hands each node to the focused modules:
//! [`complexity`] for decisions, [`rules`] for structural findings, [`imports`] for
//! dependency edges, [`functions`] for names and parameters, and [`python`] for
//! docstrings.

mod complexity;
mod functions;
mod imports;
mod python;
mod rules;
#[cfg(test)]
mod tests;
mod traversal;

use crate::{
    language::Language,
    lines::LineSpan,
    model::{Analysis, DiagnosticCode},
    syntax::{Grammar, Syntax},
};
use hashbrown::HashMap;
use std::{collections::TryReserveError, fmt, io, str::Utf8Error};
use tree_sitter::{Node, Parser, Tree};

/// Tree-sitter addresses source bytes with 32-bit offsets.
const MAX_SOURCE_BYTES: u64 = u32::MAX as u64;

/// Reuses one parser per language, so each worker thread pays grammar setup once.
#[derive(Default)]
pub struct Worker {
    parsers: HashMap<Language, Parser>,
}

impl Worker {
    pub fn analyze(&mut self, source: &str, language: Language) -> Result<Analysis, AnalysisError> {
        check_size(source.len() as u64)?;
        let tree = self.parse(source, language)?;
        Ok(traversal::analyze(&tree, Code::new(source, language)))
    }

    fn parse(&mut self, source: &str, language: Language) -> Result<Tree, AnalysisError> {
        let parser = self.parsers.entry(language).or_insert_with(|| {
            let mut parser = Parser::new();
            parser
                .set_language(&language.tree_sitter())
                .expect("bundled grammars match the tree-sitter ABI");
            parser
        });
        let tree = parser
            .parse(source, None)
            .ok_or(AnalysisError::ParseAborted)?;
        if tree.root_node().has_error() {
            return Err(AnalysisError::Syntax {
                line: first_error_line(tree.root_node()),
            });
        }
        Ok(tree)
    }
}

/// Rejects files the parser cannot address, before anything reads them.
pub fn check_size(bytes: u64) -> Result<(), AnalysisError> {
    if bytes > MAX_SOURCE_BYTES {
        return Err(AnalysisError::TooLarge { bytes });
    }
    Ok(())
}

fn first_error_line(root: Node<'_>) -> u32 {
    let mut node = root;
    loop {
        let mut cursor = node.walk();
        let next = node
            .children(&mut cursor)
            .find(|child| child.is_error() || child.is_missing() || child.has_error());
        match next {
            Some(child) if child.is_error() || child.is_missing() => {
                return LineSpan::of(child).start();
            }
            Some(child) => node = child,
            None => return LineSpan::of(node).start(),
        }
    }
}

/// Why a file contributed no measurements.
#[derive(Debug)]
pub enum AnalysisError {
    Read(io::Error),
    TooLarge {
        bytes: u64,
    },
    Allocation(TryReserveError),
    NotUtf8 {
        line: u32,
    },
    Syntax {
        line: u32,
    },
    /// Parsing stopped without a tree, which happens only if it is cancelled.
    ParseAborted,
}

impl AnalysisError {
    /// Locates the first invalid byte of a source that failed UTF-8 validation.
    pub fn not_utf8(bytes: &[u8], error: Utf8Error) -> Self {
        let newlines = bytes[..error.valid_up_to()]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count();
        Self::NotUtf8 {
            line: u32::try_from(newlines + 1).unwrap_or(u32::MAX),
        }
    }

    /// The line the error points at, when it points into the source.
    pub fn line(&self) -> Option<u32> {
        match self {
            Self::NotUtf8 { line } | Self::Syntax { line } => Some(*line),
            Self::Read(_) | Self::TooLarge { .. } | Self::Allocation(_) | Self::ParseAborted => {
                None
            }
        }
    }

    pub fn code(&self) -> DiagnosticCode {
        match self {
            Self::Read(_) => DiagnosticCode::ReadError,
            Self::TooLarge { .. } => DiagnosticCode::FileTooLarge,
            Self::Allocation(_) => DiagnosticCode::AllocationError,
            Self::NotUtf8 { .. } => DiagnosticCode::NotUtf8,
            Self::Syntax { .. } => DiagnosticCode::SyntaxError,
            Self::ParseAborted => DiagnosticCode::ParseAborted,
        }
    }
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(error) => write!(formatter, "read source: {error}"),
            Self::TooLarge { bytes } => write!(
                formatter,
                "source is {bytes} bytes; the parser requires files smaller than 4 GiB"
            ),
            Self::Allocation(error) => write!(formatter, "allocate source buffer: {error}"),
            Self::NotUtf8 { line } => write!(
                formatter,
                "source is not UTF-8; the first invalid byte is on line {line}"
            ),
            Self::Syntax { line } => write!(
                formatter,
                "syntax error on line {line}; file excluded from metrics"
            ),
            Self::ParseAborted => formatter.write_str("parser stopped without a syntax tree"),
        }
    }
}

impl std::error::Error for AnalysisError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) => Some(error),
            Self::Allocation(error) => Some(error),
            Self::TooLarge { .. }
            | Self::NotUtf8 { .. }
            | Self::Syntax { .. }
            | Self::ParseAborted => None,
        }
    }
}

/// The file being analyzed: its text, language, and node classification.
#[derive(Clone, Copy)]
struct Code<'source> {
    source: &'source str,
    language: Language,
    grammar: &'static Grammar,
}

impl<'source> Code<'source> {
    fn new(source: &'source str, language: Language) -> Self {
        Self {
            source,
            language,
            grammar: Grammar::of(language),
        }
    }

    fn syntax(self, node: Node<'_>) -> Syntax {
        self.grammar.syntax(node)
    }

    fn text(self, node: Node<'_>) -> &'source str {
        self.source.get(node.byte_range()).unwrap_or("")
    }
}

/// An ancestor of the node being visited.
#[derive(Clone, Copy)]
struct Parent<'tree> {
    node: Node<'tree>,
    syntax: Syntax,
}

/// What the traversal already knows about a node's position, so checks never search
/// the tree for a node's parent.
#[derive(Clone, Copy)]
struct Context<'tree> {
    parent: Option<Parent<'tree>>,
    grandparent: Option<Parent<'tree>>,
    /// The field of the parent that holds this node, if any.
    field: Option<&'static str>,
    /// No earlier sibling is anything other than a comment.
    first_statement: bool,
}
