//! Folder/file naming templates, e.g. `{project}/{date}/{camera}/{category}`.
//!
//! Every rendered segment is sanitized and validated, so a template can never
//! produce `..`, an absolute path or a forbidden name.

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

use crate::classify::Category;
use crate::error::{MfError, Result};
use crate::paths::{sanitize_segment, validate_relative};

pub const TOKENS: [&str; 13] = [
    "project",
    "date",
    "year",
    "month",
    "day",
    "camera",
    "source",
    "category",
    "ext",
    "name",
    "stem",
    "original_dir",
    "time",
];

/// Values available while rendering one file.
#[derive(Debug, Clone)]
pub struct TemplateContext<'a> {
    pub project: &'a str,
    /// Label of the source (volume name or folder name). `{camera}` falls back
    /// to it until camera metadata extraction is available.
    pub source_label: &'a str,
    pub camera: Option<&'a str>,
    pub date: Option<DateTime<Utc>>,
    pub category: Category,
    /// Original relative path in the source, `/`-separated.
    pub rel_path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    /// Folder layout, `/`-separated. May be empty (files go to the root).
    pub folders: String,
    /// File name pattern; `{name}` keeps the original name.
    pub file_name: String,
}

impl Default for Template {
    fn default() -> Self {
        Template {
            folders: "{project}/{date}/{camera}/{category}".into(),
            file_name: "{name}".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Lit(String),
    Tok(String),
}

fn parse(s: &str) -> Result<Vec<Piece>> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut name = String::new();
            loop {
                match chars.next() {
                    Some('}') => break,
                    Some(ch) => name.push(ch),
                    None => {
                        return Err(MfError::InvalidInput(format!(
                            "unclosed '{{' in template {s:?}"
                        )))
                    }
                }
            }
            if !TOKENS.contains(&name.as_str()) {
                return Err(MfError::InvalidInput(format!(
                    "unknown template token {{{name}}}"
                )));
            }
            if !lit.is_empty() {
                out.push(Piece::Lit(std::mem::take(&mut lit)));
            }
            out.push(Piece::Tok(name));
        } else if c == '}' {
            return Err(MfError::InvalidInput(format!(
                "unexpected '}}' in template {s:?}"
            )));
        } else {
            lit.push(c);
        }
    }
    if !lit.is_empty() {
        out.push(Piece::Lit(lit));
    }
    Ok(out)
}

impl Template {
    /// Check syntax up front so the UI can show errors before planning.
    pub fn validate(&self) -> Result<()> {
        for seg in self.folders.split('/').filter(|s| !s.is_empty()) {
            parse(seg)?;
        }
        if self.file_name.contains('/') || self.file_name.contains('\\') {
            return Err(MfError::InvalidInput(
                "file name pattern cannot contain '/'".into(),
            ));
        }
        let p = parse(&self.file_name)?;
        if p.is_empty() {
            return Err(MfError::InvalidInput("file name pattern is empty".into()));
        }
        // Without {name}/{stem} several files would map to the same name.
        if !p
            .iter()
            .any(|x| matches!(x, Piece::Tok(t) if t == "name" || t == "stem"))
        {
            return Err(MfError::InvalidInput(
                "file name pattern must contain {name} or {stem}".into(),
            ));
        }
        Ok(())
    }

    /// Render the destination relative path for one file.
    pub fn render(&self, ctx: &TemplateContext<'_>) -> Result<String> {
        self.validate()?;
        let mut segs: Vec<String> = Vec::new();
        for raw in self.folders.split('/').filter(|s| !s.is_empty()) {
            if raw == "{original_dir}" {
                // Expand into several segments, each sanitized.
                let dir = ctx.rel_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                segs.extend(
                    dir.split('/')
                        .filter(|s| !s.is_empty())
                        .map(sanitize_segment),
                );
                continue;
            }
            let v = render_segment(&parse(raw)?, ctx);
            if !v.is_empty() {
                segs.push(sanitize_segment(&v));
            }
        }
        segs.push(sanitize_segment(&render_segment(
            &parse(&self.file_name)?,
            ctx,
        )));
        let rel = segs.join("/");
        validate_relative(&rel)?;
        Ok(rel)
    }
}

fn render_segment(pieces: &[Piece], ctx: &TemplateContext<'_>) -> String {
    let name = ctx.rel_path.rsplit('/').next().unwrap_or(ctx.rel_path);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, e),
        _ => (name, ""),
    };
    let local = ctx.date.map(|d| d.with_timezone(&Local));
    let mut out = String::new();
    for p in pieces {
        match p {
            Piece::Lit(l) => out.push_str(l),
            Piece::Tok(t) => out.push_str(&match t.as_str() {
                "project" => ctx.project.to_string(),
                "date" => local
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "Undated".into()),
                "year" => local
                    .map(|d| d.format("%Y").to_string())
                    .unwrap_or_else(|| "Undated".into()),
                "month" => local
                    .map(|d| d.format("%m").to_string())
                    .unwrap_or_else(|| "Undated".into()),
                "day" => local
                    .map(|d| d.format("%d").to_string())
                    .unwrap_or_else(|| "Undated".into()),
                "time" => local
                    .map(|d| d.format("%H%M%S").to_string())
                    .unwrap_or_default(),
                "camera" => ctx.camera.unwrap_or(ctx.source_label).to_string(),
                "source" => ctx.source_label.to_string(),
                "category" => ctx.category.folder_name().to_string(),
                "ext" => ext.to_ascii_uppercase(),
                "name" => name.to_string(),
                "stem" => stem.to_string(),
                _ => String::new(),
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ctx<'a>(rel: &'a str, project: &'a str) -> TemplateContext<'a> {
        TemplateContext {
            project,
            source_label: "SD_A7S3",
            camera: None,
            date: Some(
                Local
                    .with_ymd_and_hms(2024, 5, 1, 12, 0, 0)
                    .unwrap()
                    .with_timezone(&Utc),
            ),
            category: Category::Video,
            rel_path: rel,
        }
    }

    #[test]
    fn default_layout() {
        let t = Template::default();
        let r = t
            .render(&ctx("PRIVATE/M4ROOT/CLIP/C0001.MP4", "Client X"))
            .unwrap();
        assert_eq!(r, "Client X/2024-05-01/SD_A7S3/Video/C0001.MP4");
    }

    #[test]
    fn hostile_values_are_sanitized() {
        let t = Template {
            folders: "{project}/{original_dir}".into(),
            file_name: "{date}_{name}".into(),
        };
        let r = t.render(&ctx("a/../b/c.mov", "../../etc")).unwrap();
        assert!(!r.split('/').any(|s| s == ".."), "{r}");
        validate_relative(&r).unwrap();
    }

    #[test]
    fn rejects_bad_templates() {
        assert!(Template {
            folders: "{nope}".into(),
            file_name: "{name}".into()
        }
        .validate()
        .is_err());
        assert!(Template {
            folders: "{project".into(),
            file_name: "{name}".into()
        }
        .validate()
        .is_err());
        assert!(Template {
            folders: "".into(),
            file_name: "fixed.mov".into()
        }
        .validate()
        .is_err());
        assert!(Template {
            folders: "".into(),
            file_name: "a/{name}".into()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn empty_project_segment_dropped() {
        let t = Template::default();
        let r = t.render(&ctx("x.mp4", "")).unwrap();
        assert_eq!(r, "2024-05-01/SD_A7S3/Video/x.mp4");
    }
}
