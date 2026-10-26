//! File identity comparison. Heuristics and proofs are kept strictly apart:
//! only equal SHA-256 digests yield [`MatchLevel::VerifiedIdentical`].

use serde::{Deserialize, Serialize};

use crate::hash::Sha256Hex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchLevel {
    /// SHA-256 digests are equal (both computed). The only proof of identity.
    VerifiedIdentical,
    /// Same size, same name and same modification time; not hashed yet.
    ProbableIdentical,
    /// Same size, but name and/or date differ; not hashed. Could be anything.
    SameSizeOnly,
    /// Same name but content proven different (size or digest differs).
    SameNameDifferentContent,
    /// Proven different and unrelated by name.
    Different,
    /// At least one side could not be read.
    Undetermined,
}

impl MatchLevel {
    pub fn is_proven_identical(self) -> bool {
        self == MatchLevel::VerifiedIdentical
    }

    pub fn label(self) -> &'static str {
        match self {
            MatchLevel::VerifiedIdentical => "Identical content (verified)",
            MatchLevel::ProbableIdentical => "Probably identical (not verified)",
            MatchLevel::SameSizeOnly => "Same size only",
            MatchLevel::SameNameDifferentContent => "Same name, different content",
            MatchLevel::Different => "Different",
            MatchLevel::Undetermined => "Cannot be determined",
        }
    }
}

/// What we know about one side of a comparison.
#[derive(Debug, Clone, Default)]
pub struct Facts<'a> {
    pub name: &'a str,
    pub size: Option<u64>,
    pub mtime_ns: Option<i64>,
    pub sha256: Option<&'a Sha256Hex>,
}

fn same_name(a: &str, b: &str) -> bool {
    crate::paths::collision_key(a) == crate::paths::collision_key(b)
}

pub fn compare(a: &Facts<'_>, b: &Facts<'_>) -> MatchLevel {
    let names = same_name(a.name, b.name);
    let (Some(sa), Some(sb)) = (a.size, b.size) else {
        return MatchLevel::Undetermined;
    };
    if sa != sb {
        return if names {
            MatchLevel::SameNameDifferentContent
        } else {
            MatchLevel::Different
        };
    }
    if let (Some(ha), Some(hb)) = (a.sha256, b.sha256) {
        return if ha == hb {
            MatchLevel::VerifiedIdentical
        } else if names {
            MatchLevel::SameNameDifferentContent
        } else {
            MatchLevel::Different
        };
    }
    // Times within 2 s are considered equal: FAT/exFAT store 2-second resolution.
    let close_time = match (a.mtime_ns, b.mtime_ns) {
        (Some(x), Some(y)) if x != 0 && y != 0 => (x - y).abs() <= 2_000_000_000,
        _ => false,
    };
    if names && close_time {
        MatchLevel::ProbableIdentical
    } else {
        MatchLevel::SameSizeOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_bytes;

    #[test]
    fn name_alone_is_never_identity() {
        let a = Facts {
            name: "C0001.MP4",
            size: Some(10),
            mtime_ns: Some(1),
            sha256: None,
        };
        let b = Facts {
            name: "C0001.MP4",
            size: Some(11),
            mtime_ns: Some(1),
            sha256: None,
        };
        assert_eq!(compare(&a, &b), MatchLevel::SameNameDifferentContent);
    }

    #[test]
    fn size_and_name_is_only_probable() {
        let a = Facts {
            name: "C0001.MP4",
            size: Some(10),
            mtime_ns: Some(5_000_000_000),
            sha256: None,
        };
        let b = Facts {
            name: "c0001.mp4",
            size: Some(10),
            mtime_ns: Some(6_000_000_000),
            sha256: None,
        };
        let l = compare(&a, &b);
        assert_eq!(l, MatchLevel::ProbableIdentical);
        assert!(!l.is_proven_identical());
    }

    #[test]
    fn hash_proves_identity_even_with_different_name() {
        let h = hash_bytes(b"x");
        let a = Facts {
            name: "A.MP4",
            size: Some(1),
            mtime_ns: None,
            sha256: Some(&h),
        };
        let b = Facts {
            name: "renamed.mp4",
            size: Some(1),
            mtime_ns: None,
            sha256: Some(&h),
        };
        assert_eq!(compare(&a, &b), MatchLevel::VerifiedIdentical);
    }

    #[test]
    fn hash_disproves_same_name_same_size() {
        let (h1, h2) = (hash_bytes(b"x"), hash_bytes(b"y"));
        let a = Facts {
            name: "A.MP4",
            size: Some(1),
            mtime_ns: Some(1),
            sha256: Some(&h1),
        };
        let b = Facts {
            name: "A.MP4",
            size: Some(1),
            mtime_ns: Some(1),
            sha256: Some(&h2),
        };
        assert_eq!(compare(&a, &b), MatchLevel::SameNameDifferentContent);
    }

    #[test]
    fn unknown_size_undetermined() {
        let a = Facts {
            name: "A",
            size: None,
            ..Default::default()
        };
        let b = Facts {
            name: "A",
            size: Some(1),
            ..Default::default()
        };
        assert_eq!(compare(&a, &b), MatchLevel::Undetermined);
    }

    #[test]
    fn same_size_different_name() {
        let a = Facts {
            name: "A",
            size: Some(1),
            mtime_ns: Some(1),
            sha256: None,
        };
        let b = Facts {
            name: "B",
            size: Some(1),
            mtime_ns: Some(1),
            sha256: None,
        };
        assert_eq!(compare(&a, &b), MatchLevel::SameSizeOnly);
    }
}
