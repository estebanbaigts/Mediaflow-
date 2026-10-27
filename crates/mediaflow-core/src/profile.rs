//! Reusable project profiles: layout, naming, exclusions, verification and
//! duplicate/conflict policy. Exclusions are always shown in the plan.

use globset::{Glob, GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};

use crate::classify::Category;
use crate::error::{MfError, Result};
use crate::template::Template;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyLevel {
    /// Hash source while copying, re-read and hash every copy before finalizing.
    Full,
    /// `Full`, plus a final pass re-reading every finalized copy from disk at the
    /// end of the session.
    Paranoid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicatePolicy {
    /// Skip a file whose identical content (SHA-256 verified at execution time)
    /// already exists at the planned destination path.
    SkipVerified,
    /// Always copy; if the planned path is taken, a suffixed name is used.
    CopyAnyway,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Same name, different content: copy under `name (2).ext`.
    KeepBothRenamed,
    /// Same name, different content: do not copy, flag for the user.
    Flag,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExclusionRule {
    /// Glob matched against the source relative path (case-insensitive),
    /// e.g. `**/.DS_Store`.
    pub pattern: String,
    /// Shown to the user next to every excluded file.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryOverride {
    pub pattern: String,
    pub category: Category,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub template: Template,
    pub exclusions: Vec<ExclusionRule>,
    pub category_overrides: Vec<CategoryOverride>,
    pub verify: VerifyLevel,
    pub duplicates: DuplicatePolicy,
    pub conflicts: ConflictPolicy,
    pub preserve_mtime: bool,
}

pub(crate) struct CompiledProfile {
    pub exclusions: Vec<(GlobMatcher, ExclusionRule)>,
    pub overrides: Vec<(GlobMatcher, Category)>,
}

fn glob(p: &str) -> Result<GlobMatcher> {
    let g: Glob = GlobBuilder::new(p)
        .case_insensitive(true)
        .literal_separator(false)
        .build()
        .map_err(|e| MfError::InvalidInput(format!("invalid pattern {p:?}: {e}")))?;
    Ok(g.compile_matcher())
}

impl Profile {
    pub(crate) fn compile(&self) -> Result<CompiledProfile> {
        self.template.validate()?;
        Ok(CompiledProfile {
            exclusions: self
                .exclusions
                .iter()
                .map(|r| Ok((glob(&r.pattern)?, r.clone())))
                .collect::<Result<_>>()?,
            overrides: self
                .category_overrides
                .iter()
                .map(|o| Ok((glob(&o.pattern)?, o.category)))
                .collect::<Result<_>>()?,
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(MfError::InvalidInput(
                "profile id must be [a-z0-9_-]".into(),
            ));
        }
        self.compile().map(|_| ())
    }
}

fn system_junk() -> Vec<ExclusionRule> {
    vec![
        ExclusionRule {
            pattern: "**/.DS_Store".into(),
            reason: "macOS folder metadata".into(),
        },
        ExclusionRule {
            pattern: "**/._*".into(),
            reason: "macOS resource-fork companion file".into(),
        },
        ExclusionRule {
            pattern: "**/Thumbs.db".into(),
            reason: "Windows thumbnail cache".into(),
        },
        ExclusionRule {
            pattern: ".Spotlight-V100/**".into(),
            reason: "macOS Spotlight index".into(),
        },
        ExclusionRule {
            pattern: ".Trashes/**".into(),
            reason: "macOS trash of the volume".into(),
        },
        ExclusionRule {
            pattern: ".fseventsd/**".into(),
            reason: "macOS file system journal".into(),
        },
        ExclusionRule {
            pattern: "System Volume Information/**".into(),
            reason: "Windows system folder".into(),
        },
    ]
}

fn base(id: &str, name: &str, description: &str, folders: &str) -> Profile {
    Profile {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        template: Template {
            folders: folders.into(),
            file_name: "{name}".into(),
        },
        exclusions: system_junk(),
        category_overrides: vec![],
        verify: VerifyLevel::Full,
        duplicates: DuplicatePolicy::SkipVerified,
        conflicts: ConflictPolicy::KeepBothRenamed,
        preserve_mtime: true,
    }
}

/// Built-in profiles. Users can derive custom ones from these.
pub fn builtin_profiles() -> Vec<Profile> {
    let mut youtube = base(
        "youtube",
        "YouTube",
        "One folder per shoot day, split by media type.",
        "{project}/{date}/{category}",
    );
    youtube.category_overrides.push(CategoryOverride {
        pattern: "**/*.lrv".into(),
        category: Category::Other,
    });

    let mut pro = base(
        "pro_video",
        "Professional video",
        "Day / camera / media type, with a final re-read of every copy.",
        "{project}/{date}/{camera}/{category}",
    );
    pro.verify = VerifyLevel::Paranoid;
    pro.conflicts = ConflictPolicy::Flag;

    let mut wedding = base(
        "wedding",
        "Wedding & events",
        "Per event day and source, keeping the card's original folders.",
        "{project}/{date}/{camera}/{original_dir}",
    );
    wedding.verify = VerifyLevel::Paranoid;

    let mut podcast = base(
        "podcast",
        "Podcast",
        "Episode / media type. WAV recordings are filed as voice-over.",
        "{project}/{category}",
    );
    podcast.category_overrides.push(CategoryOverride {
        pattern: "**/*.wav".into(),
        category: Category::Voiceover,
    });

    let photo = base(
        "photography",
        "Photography",
        "Year / date, RAW and JPEG together.",
        "{project}/{year}/{date}",
    );

    let custom = base(
        "custom",
        "Custom",
        "Mirror the source structure under the project folder.",
        "{project}/{original_dir}",
    );

    vec![youtube, pro, wedding, podcast, photo, custom]
}

pub fn builtin(id: &str) -> Option<Profile> {
    builtin_profiles().into_iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid() {
        for p in builtin_profiles() {
            p.validate().unwrap();
        }
    }

    #[test]
    fn exclusion_globs_match() {
        let p = builtin("youtube").unwrap().compile().unwrap();
        let hit = |s: &str| p.exclusions.iter().any(|(g, _)| g.is_match(s));
        assert!(hit(".DS_Store"));
        assert!(hit("DCIM/.DS_Store"));
        assert!(hit("DCIM/._C0001.MP4"));
        assert!(hit("dcim/THUMBS.DB"));
        assert!(!hit("DCIM/C0001.MP4"));
    }

    #[test]
    fn bad_profile_rejected() {
        let mut p = builtin("custom").unwrap();
        p.exclusions.push(ExclusionRule {
            pattern: "[".into(),
            reason: "x".into(),
        });
        assert!(p.validate().is_err());
        let mut p = builtin("custom").unwrap();
        p.id = "../x".into();
        assert!(p.validate().is_err());
    }
}
