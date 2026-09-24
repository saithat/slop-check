//! Supported languages, their file extensions, and their tree-sitter grammars.

use serde::Serialize;
use std::{fmt, path::Path};

/// A grammar the analyzer parses. TypeScript and TSX are separate grammars, so they are
/// reported separately and their blocks never match each other as clones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    Javascript,
    Typescript,
    Tsx,
    Rust,
    Go,
}

impl Language {
    pub const ALL: [Self; 6] = [
        Self::Python,
        Self::Javascript,
        Self::Typescript,
        Self::Tsx,
        Self::Rust,
        Self::Go,
    ];

    /// Maps a filename extension to the exact grammar used for parsing and reporting.
    pub fn detect(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "py" | "pyw" => Some(Self::Python),
            "js" | "jsx" | "mjs" | "cjs" => Some(Self::Javascript),
            "ts" | "mts" | "cts" => Some(Self::Typescript),
            "tsx" => Some(Self::Tsx),
            "rs" => Some(Self::Rust),
            "go" => Some(Self::Go),
            _ => None,
        }
    }

    pub fn tree_sitter(self) -> tree_sitter::Language {
        match self {
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Javascript => tree_sitter_javascript::LANGUAGE.into(),
            Self::Typescript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    /// Whether the dependency graph resolves this language's imports.
    pub fn resolves_imports(self) -> bool {
        match self {
            Self::Python | Self::Javascript | Self::Typescript | Self::Tsx => true,
            Self::Rust | Self::Go => false,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Python => "Python",
            Self::Javascript => "JavaScript",
            Self::Typescript => "TypeScript",
            Self::Tsx => "TSX",
            Self::Rust => "Rust",
            Self::Go => "Go",
        })
    }
}
