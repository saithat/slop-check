//! The deliberately small dependency graph between scanned files.
//!
//! Only local Python imports and relative JavaScript/TypeScript specifiers resolve.
//! Package managers, path aliases, and dynamic imports are outside this resolver.

use crate::{
    model::{Dependencies, Edge, Import, PythonImport},
    path::RelativePath,
    report::ScannedFile,
};
use hashbrown::HashMap;
use petgraph::{algo::kosaraju_scc, graphmap::DiGraphMap};
use std::{ffi::OsStr, path::Path};

pub fn analyze(files: &[ScannedFile]) -> Dependencies {
    let mut graph = DependencyGraph::new(files);
    for (index, file) in files.iter().enumerate() {
        if file.language.resolves_imports() {
            graph.add_file(index, &file.analysis.imports);
        }
    }
    graph.finish()
}

/// Graph nodes are indexes into the path-sorted `files`, which also gives every edge
/// endpoint a deterministic identity.
struct DependencyGraph<'a> {
    files: &'a [ScannedFile],
    /// The files whose imports resolve, by path.
    index: HashMap<&'a str, usize>,
    graph: DiGraphMap<usize, ()>,
    analyzed_files: u64,
    unresolved_imports: u64,
}

impl<'a> DependencyGraph<'a> {
    fn new(files: &'a [ScannedFile]) -> Self {
        let index = files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.language.resolves_imports())
            .map(|(index, file)| (file.path.as_str(), index))
            .collect();
        Self {
            files,
            index,
            graph: DiGraphMap::new(),
            analyzed_files: 0,
            unresolved_imports: 0,
        }
    }

    fn add_file(&mut self, source: usize, imports: &[Import]) {
        self.analyzed_files += 1;
        self.graph.add_node(source);
        for import in imports {
            let targets = self.resolve(&self.files[source].path, import);
            // Count unresolved import statements, not attempted file names.
            if targets.is_empty() {
                self.unresolved_imports += 1;
            }
            // The graph keeps one edge per pair, however many imports repeat it.
            for target in targets.into_iter().filter(|target| *target != source) {
                self.graph.add_edge(source, target, ());
            }
        }
    }

    fn resolve(&self, from: &RelativePath, import: &Import) -> Vec<usize> {
        let mut targets: Vec<usize> = match import {
            Import::Python(import) => python_candidates(from, import)
                .iter()
                .filter_map(|candidate| self.python_module(candidate))
                .collect(),
            Import::Specifier(specifier) => relative_target(from, specifier)
                .and_then(|target| self.javascript_module(&target))
                .into_iter()
                .collect(),
        };
        targets.sort_unstable();
        targets.dedup();
        targets
    }

    fn lookup(&self, path: &str) -> Option<usize> {
        self.index.get(path).copied()
    }

    fn python_module(&self, base: &str) -> Option<usize> {
        let package = if base.is_empty() {
            "__init__.py".to_owned()
        } else {
            format!("{base}/__init__.py")
        };
        self.lookup(&format!("{base}.py"))
            .or_else(|| self.lookup(&package))
    }

    /// Resolves a specifier the way bundlers do for local files: an exact file, the
    /// TypeScript source behind a `.js` specifier, or an inferred extension or index
    /// file. Dotted names such as `./user.service` have no script extension, so they
    /// get an inferred one.
    fn javascript_module(&self, target: &str) -> Option<usize> {
        if let Some(index) = self.lookup(target) {
            return Some(index);
        }
        let path = Path::new(target);
        match path.extension().and_then(OsStr::to_str) {
            Some(extension) if SCRIPT_EXTENSIONS.contains(&extension) => {
                source_alternatives(extension)
                    .iter()
                    .find_map(|alternative| self.lookup(path.with_extension(alternative).to_str()?))
            }
            _ => SCRIPT_EXTENSIONS.iter().find_map(|extension| {
                self.lookup(&format!("{target}.{extension}"))
                    .or_else(|| self.lookup(&format!("{target}/index.{extension}")))
            }),
        }
    }

    fn finish(self) -> Dependencies {
        let path = |index: usize| self.files[index].path.clone();
        // Hash and graph iteration order must not reach the report, so sort here.
        let mut edges: Vec<_> = self
            .graph
            .all_edges()
            .map(|(from, to, ())| (from, to))
            .collect();
        edges.sort_unstable();
        // Each strongly connected component with more than one file is a set of
        // mutually reachable files: evidence of a cycle, not one chosen path. Self-imports
        // have no edge, so single-file components are never cycles.
        let mut cycles: Vec<Vec<RelativePath>> = kosaraju_scc(&self.graph)
            .into_iter()
            .filter(|component| component.len() > 1)
            .map(|component| {
                let mut paths: Vec<_> = component.into_iter().map(path).collect();
                paths.sort();
                paths
            })
            .collect();
        cycles.sort();
        Dependencies {
            analyzed_files: self.analyzed_files,
            resolved_edges: edges
                .into_iter()
                .map(|(from, to)| Edge {
                    from: path(from),
                    to: path(to),
                })
                .collect(),
            cycles,
            unresolved_imports: self.unresolved_imports,
        }
    }
}

/// Script extensions in the order they are tried for an extensionless specifier.
const SCRIPT_EXTENSIONS: [&str; 8] = ["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"];

/// TypeScript sources that compile to a specifier's JavaScript extension.
fn source_alternatives(extension: &str) -> &'static [&'static str] {
    match extension {
        "js" => &["ts", "tsx"],
        "mjs" => &["mts"],
        "cjs" => &["cts"],
        _ => &[],
    }
}

/// Module paths an import may name, relative to the scan root. Absolute imports may start
/// at the root or at the common `src/` layout; leading dots climb from the importing
/// file's package. Names after `from ... import` may themselves be submodules.
fn python_candidates(from: &RelativePath, import: &PythonImport) -> Vec<String> {
    let roots: Vec<Vec<&str>> = if import.level == 0 {
        vec![vec![], vec!["src"]]
    } else {
        let mut package: Vec<&str> = from.parent_segments().collect();
        for _ in 1..import.level {
            if package.pop().is_none() {
                // The import climbs above the scan root.
                return vec![];
            }
        }
        vec![package]
    };
    let mut candidates = vec![];
    for root in roots {
        let mut base = root;
        base.extend(import.module.iter().map(String::as_str));
        for name in &import.names {
            let mut submodule = base.clone();
            submodule.extend(name.split('.'));
            candidates.push(submodule.join("/"));
        }
        candidates.push(base.join("/"));
    }
    candidates
}

/// The scan-relative path a relative specifier such as `../lib/util` names, or `None`
/// for a package specifier or one that climbs above the scan root.
fn relative_target(from: &RelativePath, specifier: &str) -> Option<String> {
    if !specifier.starts_with('.') {
        return None;
    }
    let mut segments: Vec<&str> = from.parent_segments().collect();
    for segment in specifier.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            name => segments.push(name),
        }
    }
    Some(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use crate::{
        model::{Dependencies, Edge},
        path::RelativePath,
        testing::report,
    };
    use std::path::Path;

    fn dependencies(files: &[(&str, &str)]) -> Dependencies {
        report(files).dependencies
    }

    fn path(path: &str) -> RelativePath {
        RelativePath::new(Path::new(path)).unwrap()
    }

    fn edge(from: &str, to: &str) -> Edge {
        Edge {
            from: path(from),
            to: path(to),
        }
    }

    #[test]
    fn relative_imports_form_cycles() {
        let found = dependencies(&[
            ("app/a.ts", "import {b} from './b.js'; export const a = 1;"),
            ("app/b.ts", "import {a} from './a.js'; export const b = 2;"),
            ("pkg/a.py", "from . import b\n"),
            ("pkg/b.py", "from . import a\n"),
        ]);
        assert_eq!(
            found.cycles,
            [
                [path("app/a.ts"), path("app/b.ts")],
                [path("pkg/a.py"), path("pkg/b.py")]
            ]
        );
        assert_eq!(found.resolved_edges.len(), 4);
        assert_eq!(found.analyzed_files, 4);
    }

    #[test]
    fn dotted_module_names_get_an_inferred_extension() {
        let found = dependencies(&[
            (
                "app/app.module.ts",
                "import {UserService} from './user.service';\n",
            ),
            (
                "app/user.service.ts",
                "import {AppModule} from './app.module';\n",
            ),
        ]);
        assert_eq!(
            found.resolved_edges,
            [
                edge("app/app.module.ts", "app/user.service.ts"),
                edge("app/user.service.ts", "app/app.module.ts")
            ]
        );
        assert_eq!(found.unresolved_imports, 0);
    }

    #[test]
    fn specifiers_resolve_index_files_and_parent_directories() {
        let found = dependencies(&[
            ("lib/index.js", "module.exports = {};\n"),
            (
                "src/main.js",
                "const lib = require('../lib'); import('./missing');\n",
            ),
            ("src/view.jsx", "import React from 'react';\n"),
        ]);
        assert_eq!(found.resolved_edges, [edge("src/main.js", "lib/index.js")]);
        assert_eq!(found.unresolved_imports, 2);
    }

    #[test]
    fn python_imports_resolve_absolute_relative_and_src_layouts() {
        let found = dependencies(&[
            ("src/pkg/__init__.py", ""),
            (
                "src/pkg/core.py",
                "from ..pkg import util\nimport pkg.util\n",
            ),
            ("src/pkg/util.py", "from . import core\nimport os\n"),
        ]);
        assert_eq!(
            found.resolved_edges,
            [
                edge("src/pkg/core.py", "src/pkg/__init__.py"),
                edge("src/pkg/core.py", "src/pkg/util.py"),
                edge("src/pkg/util.py", "src/pkg/__init__.py"),
                edge("src/pkg/util.py", "src/pkg/core.py"),
            ]
        );
        assert_eq!(found.unresolved_imports, 1);
    }
}
