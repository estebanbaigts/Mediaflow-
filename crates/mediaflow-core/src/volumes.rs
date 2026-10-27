//! Mounted volumes and free space (best effort; information only).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    pub name: String,
    pub mount_point: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub removable: bool,
    pub read_only: bool,
}

/// List mounted volumes, skipping pseudo file systems.
pub fn list_volumes() -> Vec<Volume> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut out: Vec<Volume> = disks
        .list()
        .iter()
        .filter(|d| d.total_space() > 0)
        .filter(|d| {
            let fs = d.file_system().to_string_lossy().to_ascii_lowercase();
            !matches!(
                fs.as_str(),
                "overlay" | "tmpfs" | "devtmpfs" | "squashfs" | "proc" | "sysfs" | "autofs"
            )
        })
        .map(|d| Volume {
            name: d.name().to_string_lossy().into_owned(),
            mount_point: d.mount_point().to_string_lossy().into_owned(),
            file_system: d.file_system().to_string_lossy().into_owned(),
            total_bytes: d.total_space(),
            available_bytes: d.available_space(),
            removable: d.is_removable(),
            read_only: d.is_read_only(),
        })
        .collect();
    out.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    out.dedup_by(|a, b| a.mount_point == b.mount_point);
    out
}

/// Free space for the volume holding `path`.
pub fn available_space(path: &std::path::Path) -> Option<u64> {
    fs4::available_space(path).ok()
}

/// A removable volume that appeared after the watcher started. The UI refers
/// to it only by `id`, an opaque value minted here; it can never supply a
/// path of its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedVolume {
    pub id: String,
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
}

/// Tracks removable volumes between polls. Volumes present when the
/// registry is created are the baseline and are never announced.
#[derive(Debug, Default)]
pub struct DetectionRegistry {
    known: std::collections::HashSet<String>,
    detected: std::collections::HashMap<String, DetectedVolume>,
}

impl DetectionRegistry {
    pub fn new(baseline: &[Volume]) -> Self {
        DetectionRegistry {
            known: baseline
                .iter()
                .filter(|v| v.removable)
                .map(|v| v.mount_point.clone())
                .collect(),
            detected: Default::default(),
        }
    }

    /// Feed the current volume list; returns the newly appeared removable
    /// volumes. Volumes that disappeared are forgotten (their ids stop working).
    pub fn update(&mut self, now: &[Volume]) -> Vec<DetectedVolume> {
        let present: std::collections::HashSet<String> = now
            .iter()
            .filter(|v| v.removable)
            .map(|v| v.mount_point.clone())
            .collect();
        let mut fresh = Vec::new();
        for v in now
            .iter()
            .filter(|v| v.removable && !self.known.contains(&v.mount_point))
        {
            let d = DetectedVolume {
                id: uuid::Uuid::new_v4().to_string(),
                name: if v.name.is_empty() {
                    v.mount_point.clone()
                } else {
                    v.name.clone()
                },
                mount_point: v.mount_point.clone(),
                total_bytes: v.total_bytes,
            };
            self.detected.insert(d.id.clone(), d.clone());
            fresh.push(d);
        }
        self.detected
            .retain(|_, d| present.contains(&d.mount_point));
        self.known = present;
        fresh
    }

    pub fn get(&self, id: &str) -> Option<&DetectedVolume> {
        self.detected.get(id)
    }

    pub fn all(&self) -> Vec<DetectedVolume> {
        self.detected.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vol(mount: &str, removable: bool) -> Volume {
        Volume {
            name: mount.trim_start_matches('/').into(),
            mount_point: mount.into(),
            file_system: "exfat".into(),
            total_bytes: 64,
            available_bytes: 32,
            removable,
            read_only: false,
        }
    }

    #[test]
    fn baseline_is_not_announced_and_ids_expire_on_removal() {
        let mut r = DetectionRegistry::new(&[vol("/Volumes/OLD", true), vol("/", false)]);
        assert!(r
            .update(&[vol("/Volumes/OLD", true), vol("/", false)])
            .is_empty());
        let fresh = r.update(&[
            vol("/Volumes/OLD", true),
            vol("/Volumes/CARD", true),
            vol("/Volumes/HD", false),
        ]);
        assert_eq!(fresh.len(), 1, "only removable, only new");
        let id = fresh[0].id.clone();
        assert_eq!(r.get(&id).unwrap().mount_point, "/Volumes/CARD");
        // Still present: not announced twice.
        assert!(r.update(&[vol("/Volumes/CARD", true)]).is_empty());
        // Unplugged: the id no longer resolves.
        r.update(&[]);
        assert!(r.get(&id).is_none());
        // Plugged again: a new id.
        let again = r.update(&[vol("/Volumes/CARD", true)]);
        assert_ne!(again[0].id, id);
    }

    #[test]
    fn unknown_ids_never_resolve() {
        let mut r = DetectionRegistry::new(&[]);
        r.update(&[vol("/Volumes/CARD", true)]);
        for id in ["", "/Volumes/CARD", "../../etc", "CARD"] {
            assert!(r.get(id).is_none(), "{id:?}");
        }
    }
}
