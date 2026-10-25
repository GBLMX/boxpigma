use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use super::CacheManager;
use crate::{
    state::{ContentState, PaginationInfo},
    utils::fs::write_atomic,
};

#[derive(Serialize, Deserialize)]
struct ContentCacheEntry {
    data: ContentState,
    #[serde(default)]
    pagination: Option<PaginationInfo>,
    cached_at: u64,
}

#[derive(Serialize)]
struct ContentCacheEntryRef<'a> {
    data: &'a ContentState,
    pagination: Option<&'a PaginationInfo>,
    cached_at: u64,
}

impl CacheManager {
    fn content_path(&self, api: &str) -> PathBuf {
        self.content_dir.join(format!("{}.json", api))
    }

    pub async fn load_content_cache_async(
        &self,
        api: &str,
        ttl_secs: u64,
    ) -> Option<(ContentState, Option<PaginationInfo>)> {
        let path = self.content_path(api);
        tokio::task::spawn_blocking(move || {
            let data = fs::read_to_string(path).ok()?;
            let entry: ContentCacheEntry = serde_json::from_str(&data).ok()?;
            let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
            // A future timestamp (the wall clock moved backwards) is expired:
            // `now - cached_at` would otherwise underflow in debug builds.
            if entry.cached_at > now || now - entry.cached_at > ttl_secs {
                return None;
            }
            Some((entry.data, entry.pagination))
        })
        .await
        .ok()
        .flatten()
    }

    pub fn save_content_cache(
        &self,
        api: &str,
        content: &ContentState,
        pagination: Option<&PaginationInfo>,
    ) {
        let cached_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = ContentCacheEntryRef {
            data: content,
            pagination,
            cached_at,
        };
        match serde_json::to_string(&entry) {
            Ok(json) => {
                if let Err(e) = write_atomic(&self.content_path(api), json.as_bytes()) {
                    log::warn!("Failed to write content cache for {api}: {e}");
                }
            }
            Err(e) => {
                log::warn!("Failed to serialize content cache for {api}: {e}");
            }
        }
    }

    pub fn remove_content_cache(&self, api: &str) {
        match fs::remove_file(self.content_path(api)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("Failed to remove content cache for {api}: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::cache::CacheManager;

    fn scratch(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "boxpigma-content-test-{}-{label}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn a_future_cache_timestamp_is_treated_as_expired() {
        let base = scratch("future");
        let cache = CacheManager::new(base.join("downloads"), base.clone(), "{}".into());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_secs();
        let path = cache.content_path("api");
        fs::create_dir_all(path.parent().expect("cache parent")).expect("cache dir");
        let entry = ContentCacheEntry {
            data: ContentState::Empty,
            pagination: None,
            cached_at: now + 3_600,
        };
        fs::write(&path, serde_json::to_vec(&entry).expect("cache json")).expect("cache file");

        assert!(
            cache.load_content_cache_async("api", 60).await.is_none(),
            "a cache written by a clock ahead of ours must not underflow or load"
        );
        let _ = fs::remove_dir_all(base);
    }
}
