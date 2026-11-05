//! Deterministic file discovery and parallel per-file analysis.

use crate::{
    analyze::{self, AnalysisError, Worker},
    language::Language,
    model::{Analysis, Diagnostic, DiagnosticCode, Report, ScanScope},
    path::RelativePath,
    report::{ScannedFile, assemble},
};
use anyhow::{Context, Result, ensure};
use ignore::WalkBuilder;
use rayon::prelude::*;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

// Parsers are costly to create and cannot be shared mutably, so each Rayon worker keeps
// its own and reuses them without locking.
thread_local! { static WORKER: RefCell<Worker> = RefCell::new(Worker::default()); }

/// The combined size of files analyzed at once stays under this estimate; a larger file
/// runs alone.
const ACTIVE_INPUT_BYTES: u64 = 64 * 1024 * 1024;

/// Scans a file or directory. Files that cannot be analyzed become diagnostics, which
/// mark the report incomplete without discarding the other files' measurements.
pub fn scan(path: &Path) -> Result<Report> {
    let Discovery {
        scope,
        sources,
        mut diagnostics,
    } = discover(path)?;
    let mut files = Vec::with_capacity(sources.len());
    for result in analyze_all(&sources)? {
        match result {
            Ok(file) => files.push(file),
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    // Discovery and analysis report diagnostics in different orders; sort them once here.
    diagnostics.sort_by(|a, b| (&a.path, &a.message, a.code).cmp(&(&b.path, &b.message, b.code)));
    // Assembly runs two independent tasks and does not need every parsing worker.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(worker_count(sources.len()).min(2))
        .build()?;
    Ok(pool.install(|| assemble(files, diagnostics, scope)))
}

/// A discovered file with a supported extension.
struct SourceFile {
    path: PathBuf,
    relative: RelativePath,
    language: Language,
}

struct Discovery {
    scope: ScanScope,
    /// In path order, which is the report's file order.
    sources: Vec<SourceFile>,
    diagnostics: Vec<Diagnostic>,
}

fn discover(path: &Path) -> Result<Discovery> {
    ensure!(
        path.exists(),
        "scan path does not exist: {}",
        path.display()
    );
    let input = path.canonicalize().context("resolve scan path")?;
    let scope = scan_scope(path, &input);
    let root = if input.is_file() {
        input.parent().context("file parent")?
    } else {
        &input
    };
    let mut sources = vec![];
    let mut diagnostics = vec![];
    for entry in source_walker(&input) {
        match Discovered::from_walk(entry, root) {
            Discovered::Source(source) => sources.push(source),
            Discovered::Problem(diagnostic) => diagnostics.push(diagnostic),
            Discovered::Ignored => {}
        }
    }
    sources.sort_unstable_by(|a, b| a.path.cmp(&b.path));
    if sources.is_empty() {
        diagnostics.push(Diagnostic {
            code: DiagnosticCode::NoSupportedFiles,
            path: None,
            line: None,
            message: "no supported source files found".to_owned(),
        });
    }
    Ok(Discovery {
        scope,
        sources,
        diagnostics,
    })
}

/// What one walked entry contributes to a scan.
enum Discovered {
    Source(SourceFile),
    Problem(Diagnostic),
    /// A directory, or a file in an unsupported language.
    Ignored,
}

impl Discovered {
    fn from_walk(entry: Result<ignore::DirEntry, ignore::Error>, root: &Path) -> Self {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Self::Problem(discovery_error(&error, root)),
        };
        let language = Language::detect(entry.path())
            .filter(|_| entry.file_type().is_some_and(|kind| kind.is_file()));
        let Some(language) = language else {
            return Self::Ignored;
        };
        let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
        match RelativePath::new(relative) {
            Some(relative) => Self::Source(SourceFile {
                path: entry.into_path(),
                relative,
                language,
            }),
            None => Self::Problem(Diagnostic {
                code: DiagnosticCode::NonUtf8Path,
                path: Some(RelativePath::lossy(relative)),
                line: None,
                message: "source paths must be UTF-8; file skipped".to_owned(),
            }),
        }
    }
}

/// A walk failure, located relative to the scan root. Its message omits the absolute
/// path, so the report does not depend on the checkout directory.
fn discovery_error(error: &ignore::Error, root: &Path) -> Diagnostic {
    let mut location = None;
    let mut cause = error;
    loop {
        match cause {
            ignore::Error::WithPath { path, err } => {
                location.get_or_insert(path);
                cause = err;
            }
            ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
                cause = err;
            }
            _ => break,
        }
    }
    let path = location
        .and_then(|path| path.strip_prefix(root).ok())
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(RelativePath::lossy);
    // Walk I/O errors repeat the absolute path in their text; keep only the OS reason.
    let message = match cause {
        ignore::Error::Io(io) => io.raw_os_error().map_or_else(
            || io.kind().to_string(),
            |code| std::io::Error::from_raw_os_error(code).to_string(),
        ),
        other => other.to_string(),
    };
    Diagnostic {
        code: DiagnosticCode::DiscoveryError,
        path,
        line: None,
        message,
    }
}

/// `requested` is the path as given and `input` its canonical form. A symlink is
/// labelled by where the link is, not where its target is.
fn scan_scope(requested: &Path, input: &Path) -> ScanScope {
    let is_file = input.is_file();
    let link = requested
        .is_symlink()
        .then(|| {
            let parent = requested.parent().filter(|p| !p.as_os_str().is_empty());
            Some(
                parent
                    .unwrap_or(Path::new("."))
                    .canonicalize()
                    .ok()?
                    .join(requested.file_name()?),
            )
        })
        .flatten();
    let named = link.as_deref().unwrap_or(input);
    let start = if is_file {
        named.parent().unwrap_or(named)
    } else {
        named
    };
    // Prefer a repository-relative label. Absolute paths would make otherwise identical
    // reports depend on the machine or checkout directory.
    let label = match start.ancestors().find(|path| path.join(".git").exists()) {
        Some(repository) => named
            .strip_prefix(repository)
            .ok()
            .filter(|path| !path.as_os_str().is_empty()),
        None => named.file_name().map(Path::new),
    };
    let path = RelativePath::lossy(label.unwrap_or(Path::new(".")));
    if is_file {
        ScanScope::File { path }
    } else {
        ScanScope::Directory { path }
    }
}

fn source_walker(input: &Path) -> ignore::Walk {
    let mut walker = WalkBuilder::new(input);
    // Local .gitignore and .ignore files apply; parent and user-global rules do not, so a
    // requested tree scans the same way on every machine. The fixed directory list skips
    // common generated and dependency trees.
    walker
        .hidden(false)
        .follow_links(false)
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        .require_git(false)
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_some_and(|kind| kind.is_dir())
                || !matches!(
                    entry.file_name().to_str(),
                    Some(
                        ".git"
                            | ".slop-check"
                            | "node_modules"
                            | "target"
                            | "vendor"
                            | "dist"
                            | "build"
                            | ".venv"
                            | "venv"
                            | "__pycache__"
                            | ".tox"
                            | ".mypy_cache"
                    )
                )
        });
    walker.build()
}

fn worker_count(files: usize) -> usize {
    // A local pool keeps environment variables from becoming hidden controls. More
    // workers than files would only add scheduling and parser overhead.
    let available = std::thread::available_parallelism().map_or(1, usize::from);
    available.min(files.max(1))
}

/// Workers for files of about `size` bytes: sizes round up to a power of two, and the
/// band's workers together stay under [`ACTIVE_INPUT_BYTES`].
fn band_workers(size: u64, available: usize) -> usize {
    let rounded = size.max(1).checked_next_power_of_two().unwrap_or(u64::MAX);
    available.min((ACTIVE_INPUT_BYTES / rounded).max(1) as usize)
}

/// Analyzes every source, returning results in discovery order.
fn analyze_all(sources: &[SourceFile]) -> Result<Vec<Result<ScannedFile, Diagnostic>>> {
    let available = worker_count(sources.len());
    let mut bands = BTreeMap::<usize, Vec<(usize, &SourceFile)>>::new();
    for (index, source) in sources.iter().enumerate() {
        let size = source
            .path
            .metadata()
            .map_or(u64::MAX, |metadata| metadata.len());
        bands
            .entry(band_workers(size, available))
            .or_default()
            .push((index, source));
    }
    let mut results = Vec::with_capacity(sources.len());
    // Bands with fewer workers hold larger files and run first. Each band's pool is
    // retired before the next, so later files do not spread large allocator arenas
    // across every worker.
    for (workers, jobs) in bands {
        let analyzed: Vec<_> = rayon::ThreadPoolBuilder::new()
            .num_threads(workers.min(jobs.len()))
            .build_scoped(
                |thread| thread.run(),
                |pool| {
                    pool.install(|| {
                        jobs.par_iter()
                            .map(|&(index, source)| (index, analyze_source(source)))
                            .collect()
                    })
                },
            )?;
        results.extend(analyzed);
    }
    // Bands change execution order; restore discovery order before aggregation.
    results.sort_unstable_by_key(|(index, _)| *index);
    Ok(results.into_iter().map(|(_, result)| result).collect())
}

fn analyze_source(source: &SourceFile) -> Result<ScannedFile, Diagnostic> {
    match read_and_analyze(&source.path, source.language) {
        Ok(analysis) => Ok(ScannedFile {
            path: source.relative.clone(),
            language: source.language,
            analysis,
        }),
        Err(error) => Err(Diagnostic {
            code: error.code(),
            path: Some(source.relative.clone()),
            line: error.line(),
            message: error.to_string(),
        }),
    }
}

fn read_and_analyze(path: &Path, language: Language) -> Result<Analysis, AnalysisError> {
    let mut file = File::open(path).map_err(AnalysisError::Read)?;
    let bytes = file.metadata().map_err(AnalysisError::Read)?.len();
    analyze::check_size(bytes)?;
    let capacity = usize::try_from(bytes).map_err(|_| AnalysisError::TooLarge { bytes })?;
    let mut source = Vec::new();
    // No policy caps file size. Reserving fallibly turns an ordinary allocation failure
    // into a diagnostic for this file.
    source
        .try_reserve_exact(capacity)
        .map_err(AnalysisError::Allocation)?;
    file.read_to_end(&mut source).map_err(AnalysisError::Read)?;
    let source = String::from_utf8(source)
        .map_err(|error| AnalysisError::not_utf8(error.as_bytes(), error.utf8_error()))?;
    WORKER.with(|worker| worker.borrow_mut().analyze(&source, language))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_bands_limit_concurrent_input_without_excluding_large_files() {
        const MIB: u64 = 1024 * 1024;
        for (size, expected) in [
            (0, 32),
            (2 * MIB, 32),
            (2 * MIB + 1, 16),
            (32 * MIB, 2),
            (32 * MIB + 1, 1),
            (u64::MAX, 1),
        ] {
            assert_eq!(band_workers(size, 32), expected);
        }
        assert_eq!(band_workers(1, 3), 3);
        assert_eq!(band_workers(u64::MAX, 1), 1);
    }
}
