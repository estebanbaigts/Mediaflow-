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
