//! Media categorization from extension + file signature ("magic bytes").
//!
//! An extension alone is not proof of type: we read the first bytes and report
//! whether the signature confirms, contradicts, or cannot judge the extension.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Video,
    Audio,
    Photo,
    Music,
    Voiceover,
    Graphics,
    Subtitles,
    Lut,
    EditProject,
    Other,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::Video,
        Category::Audio,
        Category::Photo,
        Category::Music,
        Category::Voiceover,
        Category::Graphics,
        Category::Subtitles,
        Category::Lut,
        Category::EditProject,
        Category::Other,
    ];

    /// Default folder name used by templates (`{category}`).
    pub fn folder_name(self) -> &'static str {
        match self {
            Category::Video => "Video",
            Category::Audio => "Audio",
            Category::Photo => "Photos",
            Category::Music => "Music",
            Category::Voiceover => "Voiceover",
            Category::Graphics => "Graphics",
            Category::Subtitles => "Subtitles",
            Category::Lut => "LUTs",
            Category::EditProject => "Projects",
            Category::Other => "Other",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Video => "video",
            Category::Audio => "audio",
            Category::Photo => "photo",
            Category::Music => "music",
            Category::Voiceover => "voiceover",
            Category::Graphics => "graphics",
            Category::Subtitles => "subtitles",
            Category::Lut => "lut",
            Category::EditProject => "edit_project",
            Category::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// How much we trust the category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeConfidence {
    /// Signature matches the family implied by the extension.
    Confirmed,
    /// Extension known, format has no reliable signature (text formats, some RAW).
    ExtensionOnly,
    /// Signature contradicts the extension. Flagged to the user.
    Mismatch,
    /// Unknown extension and no recognizable signature.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    IsoBmff, // mp4/mov/heic/cr3/m4a…
    Riff,    // avi/wav/webp
    Matroska,
    Mpeg2Ts,
    Mxf,
    Jpeg,
    Png,
    Gif,
    Tiff, // tif + many RAW (nef, arw, dng, cr2)
    Flac,
    Mp3,
    Ogg,
    Aiff,
    Pdf,
    Psd,
    Text, // srt/vtt/cube/xml-based projects: no signature
    Zip,  // some project formats
}

fn family_for_ext(ext: &str) -> Option<(Category, &'static [Family])> {
    use Category::*;
    use Family::*;
    Some(match ext {
        "mp4" | "mov" | "m4v" | "3gp" | "lrv" => (Video, &[IsoBmff]),
        "mxf" => (Video, &[Mxf]),
        "mts" | "m2ts" | "ts" => (Video, &[Mpeg2Ts]),
        "avi" => (Video, &[Riff]),
        "mkv" | "webm" => (Video, &[Matroska]),
        "braw" | "r3d" | "insv" | "360" => (Video, &[]),
        "wav" => (Audio, &[Riff]),
        "bwf" => (Audio, &[Riff]),
        "aif" | "aiff" => (Audio, &[Aiff]),
        "flac" => (Audio, &[Flac]),
        "mp3" => (Music, &[Mp3]),
        "m4a" | "aac" => (Music, &[IsoBmff]),
        "ogg" | "opus" => (Audio, &[Ogg]),
        "jpg" | "jpeg" => (Photo, &[Jpeg]),
        "heic" | "heif" | "cr3" => (Photo, &[IsoBmff]),
        "png" => (Graphics, &[Png]),
        "gif" => (Graphics, &[Gif]),
        "webp" => (Graphics, &[Riff]),
        "tif" | "tiff" | "dng" | "nef" | "arw" | "cr2" | "orf" | "rw2" | "pef" => (Photo, &[Tiff]),
        "raf" => (Photo, &[]),
        "psd" | "psb" => (Graphics, &[Psd]),
        "ai" | "pdf" => (Graphics, &[Pdf]),
        "svg" => (Graphics, &[Text]),
        "srt" | "vtt" | "ass" | "ssa" | "scc" | "stl" => (Subtitles, &[Text]),
        "cube" | "3dl" | "look" | "cdl" | "ccc" => (Lut, &[Text]),
        "prproj" | "drp" | "fcpxml" | "aep" | "veg" | "otio" | "edl" | "xml" => {
            (EditProject, &[Text, Zip])
        }
        "fcpbundle" | "fcpxmld" => (EditProject, &[]),
        _ => return None,
    })
}

fn sniff(head: &[u8]) -> Option<Family> {
    use Family::*;
    let starts = |p: &[u8]| head.starts_with(p);
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return Some(IsoBmff);
    }
    // QuickTime files without ftyp start with an atom such as moov/mdat/wide/free.
    if head.len() >= 8 && matches!(&head[4..8], b"moov" | b"mdat" | b"wide" | b"free" | b"skip") {
        return Some(IsoBmff);
    }
    if starts(b"RIFF") || starts(b"RF64") {
        return Some(Riff);
    }
    if starts(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(Matroska);
    }
    if starts(&[0x06, 0x0E, 0x2B, 0x34]) {
        return Some(Mxf);
    }
    if starts(&[0xFF, 0xD8, 0xFF]) {
        return Some(Jpeg);
    }
    if starts(&[0x89, b'P', b'N', b'G']) {
        return Some(Png);
    }
    if starts(b"GIF8") {
        return Some(Gif);
    }
    if starts(b"II*\0") || starts(b"MM\0*") || starts(b"IIRO") || starts(b"IIU\0") {
        return Some(Tiff);
    }
    if starts(b"fLaC") {
        return Some(Flac);
    }
    if starts(b"ID3") || (head.len() >= 2 && head[0] == 0xFF && head[1] & 0xE0 == 0xE0) {
        return Some(Mp3);
    }
    if starts(b"OggS") {
        return Some(Ogg);
    }
    if starts(b"FORM") {
        return Some(Aiff);
    }
    if starts(b"%PDF") {
        return Some(Pdf);
    }
    if starts(b"8BPS") {
        return Some(Psd);
    }
    if starts(b"PK\x03\x04") {
        return Some(Zip);
    }
    // MPEG-TS: sync byte 0x47 every 188 bytes (M2TS has a 4-byte prefix → 192).
    if head.len() >= 189 && head[0] == 0x47 && head[188] == 0x47 {
        return Some(Mpeg2Ts);
    }
    if head.len() >= 197 && head[4] == 0x47 && head[196] == 0x47 {
        return Some(Mpeg2Ts);
    }
    None
}

fn looks_textual(head: &[u8]) -> bool {
    !head.is_empty() && std::str::from_utf8(head).is_ok_and(|s| !s.contains('\0'))
        || head.is_empty()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classification {
    pub category: Category,
    pub confidence: TypeConfidence,
}

/// Classify from extension and the first bytes of the file.
pub fn classify_bytes(file_name: &str, head: &[u8]) -> Classification {
    let ext = Path::new(file_name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let sniffed = sniff(head);
    match family_for_ext(&ext) {
        Some((category, fams)) => {
            let confidence = if fams.is_empty() {
                TypeConfidence::ExtensionOnly
            } else if fams.contains(&Family::Text) && sniffed.is_none() {
                if looks_textual(&head[..head.len().min(512)]) {
                    TypeConfidence::ExtensionOnly
                } else {
                    TypeConfidence::Mismatch
                }
            } else {
                match sniffed {
                    Some(f) if fams.contains(&f) => TypeConfidence::Confirmed,
                    Some(_) => TypeConfidence::Mismatch,
                    // Truncated / zero-filled headers: we can't confirm, but empty files are not a mismatch.
                    None if head.is_empty() => TypeConfidence::ExtensionOnly,
                    None => TypeConfidence::Mismatch,
                }
            };
            Classification {
                category,
                confidence,
            }
        }
        None => {
            // Unknown extension: a recognizable signature still gives a hint,
            // but we keep the file under Other and say we don't know.
            Classification {
                category: Category::Other,
                confidence: TypeConfidence::Unknown,
            }
        }
    }
}

/// Read up to 512 bytes and classify. I/O errors yield `Unknown` (the scanner
/// records the read error separately).
pub fn classify_file(path: &Path) -> Classification {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut head = [0u8; 512];
    let n = File::open(path)
        .and_then(|mut f| read_up_to(&mut f, &mut head))
        .unwrap_or(0);
    classify_bytes(&name, &head[..n])
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mp4_head() -> Vec<u8> {
        let mut v = vec![0, 0, 0, 0x18];
        v.extend_from_slice(b"ftypmp42");
        v.resize(64, 0);
        v
    }

    #[test]
    fn confirmed_video() {
        let c = classify_bytes("C0001.MP4", &mp4_head());
        assert_eq!(
            c,
            Classification {
                category: Category::Video,
                confidence: TypeConfidence::Confirmed
            }
        );
    }

    #[test]
    fn mismatch_detected() {
        // A JPEG renamed to .mov
        let c = classify_bytes(
            "clip.mov",
            &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        assert_eq!(c.category, Category::Video);
        assert_eq!(c.confidence, TypeConfidence::Mismatch);
    }

    #[test]
    fn text_formats() {
        let c = classify_bytes("subs.srt", b"1\n00:00:01,000 --> 00:00:02,000\nHello\n");
        assert_eq!(
            c,
            Classification {
                category: Category::Subtitles,
                confidence: TypeConfidence::ExtensionOnly
            }
        );
        let c = classify_bytes("look.cube", &[0, 1, 2, 0, 0xFF, 0xFE]);
        assert_eq!(c.confidence, TypeConfidence::Mismatch);
    }

    #[test]
    fn unknown_stays_other() {
        let c = classify_bytes("notes.xyz", &mp4_head());
        assert_eq!(
            c,
            Classification {
                category: Category::Other,
                confidence: TypeConfidence::Unknown
            }
        );
        let c = classify_bytes("noext", b"");
        assert_eq!(c.category, Category::Other);
    }

    #[test]
    fn raw_tiff_family() {
        assert_eq!(
            classify_bytes("a.NEF", b"MM\0*\0\0\0\x08").confidence,
            TypeConfidence::Confirmed
        );
        assert_eq!(
            classify_bytes("a.wav", b"RIFF\0\0\0\0WAVE").category,
            Category::Audio
        );
    }

    #[test]
    fn category_roundtrip() {
        for c in Category::ALL {
            assert_eq!(Category::parse(c.as_str()), Some(c));
        }
    }
}
