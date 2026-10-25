use std::{fs, path::PathBuf};

use super::CacheManager;
use crate::utils::fs::write_atomic;

impl CacheManager {
    fn cover_path(&self, song_id: u64) -> PathBuf {
        self.covers_dir.join(format!("{song_id}.jpg"))
    }

    pub async fn load_cover_async(&self, song_id: u64) -> Option<Vec<u8>> {
        let path = self.cover_path(song_id);
        tokio::task::spawn_blocking(move || fs::read(path).ok())
            .await
            .ok()
            .flatten()
    }

    /// The file a cached cover lives in, when one has been cached — what MPRIS hands the
    /// desktop as `mpris:artUrl`.
    pub fn cached_cover(&self, song_id: u64) -> Option<PathBuf> {
        let path = self.cover_path(song_id);
        path.is_file().then_some(path)
    }

    pub fn save_cover(&self, song_id: u64, data: &[u8]) {
        if let Err(e) = write_atomic(&self.cover_path(song_id), data) {
            log::warn!("Failed to write cover cache for {song_id}: {e}");
        }
    }
}
