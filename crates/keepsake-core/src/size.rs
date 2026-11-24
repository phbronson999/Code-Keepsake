use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::fsmeasure::{self, Measured};

/// Where a repo's bytes are. The TUI draws this as a stacked bar.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeBreakdown {
    pub build: u64,
    pub git: u64,
    pub source: u64,
    pub ignored_other: u64,
}

impl SizeBreakdown {
    pub fn total(&self) -> u64 {
        self.build + self.git + self.source + self.ignored_other
    }
}

impl From<Measured> for SizeBreakdown {
    fn from(m: Measured) -> Self {
        Self {
            build: m.build,
            git: m.git,
            source: m.source,
            ignored_other: m.ignored_other,
        }
    }
}

pub fn measure(repo: &Path, ignored: &[String]) -> SizeBreakdown {
    fsmeasure::measure(repo, ignored).into()
}
