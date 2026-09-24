//! Report paths: relative to the scan root, UTF-8, and `/`-separated on every platform.

use serde::Serialize;
use std::{
    fmt,
    path::{MAIN_SEPARATOR, Path},
};

/// A path relative to the scan root, written with `/` separators on every platform so
/// reports and dependency lookups compare equal across machines.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RelativePath(String);

impl RelativePath {
    /// `None` when the path is not valid UTF-8.
    pub fn new(path: &Path) -> Option<Self> {
        path.to_str().map(Self::from_native)
    }

    /// Replaces invalid UTF-8 with U+FFFD, for labels that must exist even then.
    pub fn lossy(path: &Path) -> Self {
        Self::from_native(&path.to_string_lossy())
    }

    fn from_native(path: &str) -> Self {
        Self(if MAIN_SEPARATOR == '/' {
            path.to_owned()
        } else {
            path.replace(MAIN_SEPARATOR, "/")
        })
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The directory segments above the file, outermost first.
    pub fn parent_segments(&self) -> impl Iterator<Item = &str> {
        let parent = self.0.rsplit_once('/').map_or("", |(parent, _)| parent);
        parent.split('/').filter(|segment| !segment.is_empty())
    }
}

impl fmt::Display for RelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_segments_exclude_the_file_name() {
        let path = RelativePath::new(Path::new("app/models/user.ts")).unwrap();
        assert_eq!(
            path.parent_segments().collect::<Vec<_>>(),
            ["app", "models"]
        );
        let root = RelativePath::new(Path::new("main.py")).unwrap();
        assert_eq!(root.parent_segments().count(), 0);
    }
}
