use std::{
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use ncm_api::{NcmError, SongInfo, SongQuality};
use stream_download::{
    Settings, StreamDownload, StreamPhase, http::HttpStream, storage::temp::TempStorageProvider,
};
use tokio::sync::mpsc;

#[cfg(all(target_os = "linux", target_env = "gnu"))]
use super::engine::mem_rss_kb;
use super::{
    player::{AudioInput, AudioReader, SharedReader},
    stream_client::HeadersClient,
};
use crate::{
    cache::CacheManager,
    event::{Event, PlaybackEvent},
    service::ApiService,
};

/// Minimum bytes to pre-buffer before starting playback. Roughly ~12s of audio at 320kbps and
/// ~32s at 128kbps, leaving headroom for streams that download slower than playback. Playback
/// starts once this threshold is reached or the whole stream has finished downloading.
const PREBUFFER_BYTES: u64 = 512 * 1024;

/// Maximum wait for pre-buffering. When the network is too slow or the stream is broken, don't
/// wait forever — start playback anyway once the timeout is reached.
const PREBUFFER_TIMEOUT: Duration = Duration::from_secs(8);

/// Entries fetched per request when scanning the user's cloud disk (`/api/v1/cloud/get` pages with
/// `offset`/`limit`; it offers no search).
const CLOUD_DISK_PAGE: u32 = 100;

/// How many cloud-disk entries the fallback scans before giving up. The disk is only scanned after
/// NCM streaming failed, so the scan spends a few requests for a chance to play a song the user
/// owns; past this bound playback should fail with the original error instead of walking an
/// arbitrarily large disk.
const CLOUD_DISK_SCAN_LIMIT: u32 = 500;

/// How far an upload's duration (ms) may differ from the catalogue entry's duration and still be
/// the same recording: both sides are encoded and tagged independently, so exact equality would
/// reject the right upload. Only consulted when neither side reports `0`.
const CLOUD_MATCH_DURATION_MS: u64 = 5_000;

/// Why a song could not be turned into a playable stream.
///
/// The class drives behaviour instead of the message text: [`SourceError::Network`]
/// is retried once, every other class falls through to the cloud-disk fallback
/// immediately. Messages stay user-facing, so the UI shows the same wording as
/// before.
#[derive(Debug, thiserror::Error)]
pub(super) enum SourceError {
    /// Transient transport failure while talking to a source; worth one retry.
    #[error("网络错误: {0}")]
    Network(String),
    /// The source answered but cannot provide a stream for this song: no play URL
    /// (VIP-only / copyright / region), a trial-only URL, or an unusable response.
    #[error("{0}")]
    Unavailable(String),
    /// The stream could not be prepared locally (URL parsing, HTTP stream setup,
    /// download-cache provider).
    #[error("{0}")]
    Stream(String),
}

/// Whether a failed NCM attempt should be retried instead of falling back to the
/// cloud disk: only a transport failure is worth a second try, and only once.
/// Every other class (no play URL) would fail the same way again.
fn should_retry_ncm(error: &SourceError, attempt: u32) -> bool {
    matches!(error, SourceError::Network(_)) && attempt < 1
}

/// Decide whether the stream progress callback should record the download-cache entry.
///
/// The entry may only be written once the file is complete — an entry written mid-download
/// lists a truncated file as playable and lets eviction delete a file that is still being
/// written — and only once per stream.
fn should_record_cache(mark_cache: bool, complete: bool, sent: &AtomicBool) -> bool {
    mark_cache && complete && !sent.swap(true, Ordering::SeqCst)
}

/// The file a `Free` song plays from: `local_path` is the real path, `album` is the fallback for
/// songs that still carry their path there (and the only source when a file has no album tag,
/// which leaves `local_path` unset).
fn local_file_path(song: &SongInfo) -> &str {
    song.local_path.as_deref().unwrap_or(song.album.as_str())
}

/// Normalize a title/singer for the cloud-disk match: drop bracketed segments (`(Remastered 2011)`,
/// `[Live]`), ignore case, and keep letters and digits only, so `"Hello, World!"` and
/// `"hello world"` compare equal.
///
/// An upload carries the file's own tags, usually from a different release than the catalogue entry
/// being played, so punctuation, spacing and edition suffixes differ as a rule — a byte-exact
/// comparison would miss nearly every upload. Bracketed segments are dropped rather than compared
/// because the entry a user uploads often names the same recording without the edition marker the
/// catalogue adds; the duration check is what keeps a different recording out.
fn cloud_match_key(text: &str) -> String {
    let mut key = String::new();
    let mut depth = 0u32;
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            '(' | '[' | '（' | '【' => depth += 1,
            ')' | ']' | '）' | '】' => depth = depth.saturating_sub(1),
            _ if depth == 0 && c.is_alphanumeric() => key.push(c),
            _ => {}
        }
    }
    key
}

/// Find the cloud-disk entry that is the user's upload of `song`, if the disk holds one.
///
/// The title must agree once normalized, and the singer must overlap — one release writes `"A/B"`
/// where another writes `"A"` — because a disk can hold several uploads of the same title. Duration
/// only decides between candidates that already match: it is in ms, and NCM reports `0` for an
/// upload it could not match to a catalogue song, which must not disqualify the entry.
fn pick_cloud_match(song: &SongInfo, disk: &[SongInfo]) -> Option<u64> {
    let name = cloud_match_key(&song.name);
    if name.is_empty() {
        return None;
    }
    let singer = cloud_match_key(&song.singer);

    let mut best: Option<(u64, u64)> = None; // (duration difference, cloud song id)
    for entry in disk {
        if cloud_match_key(&entry.name) != name {
            continue;
        }
        let entry_singer = cloud_match_key(&entry.singer);
        let singers_agree = singer.is_empty()
            || entry_singer.is_empty()
            || entry_singer.contains(&singer)
            || singer.contains(&entry_singer);
        if !singers_agree {
            continue;
        }

        let difference = if entry.duration != 0 && song.duration != 0 {
            let difference = entry.duration.abs_diff(song.duration);
            if difference > CLOUD_MATCH_DURATION_MS {
                continue; // both sides timed the audio, and it is a different recording
            }
            difference
        } else {
            u64::MAX // one side does not know its duration: usable, but loses to a timed entry
        };

        if best.is_none_or(|(best_difference, _)| difference < best_difference) {
            best = Some((difference, entry.id));
        }
    }
    best.map(|(_, id)| id)
}

/// Run the streaming chain for a song whose cached copy and local file cannot be used: NCM player
/// URLs (one retry, only for a transport failure), then the user's own cloud disk.
///
/// The two resolvers are callbacks so tests can pin the order of the chain and the error it
/// reports with fake resolvers — neither can be exercised without a login and an uploaded disk
/// entry.
///
/// Adding the cloud disk must not change what a caller sees when nothing can play the song: the
/// NCM failure is reported when the disk holds no copy either.
async fn resolve_streaming<N, NF, C, CF>(
    song: &SongInfo,
    mut ncm: N,
    mut cloud: C,
) -> Result<AudioInput, SourceError>
where
    N: FnMut() -> NF,
    NF: Future<Output = Result<AudioInput, SourceError>>,
    C: FnMut() -> CF,
    CF: Future<Output = Option<AudioInput>>,
{
    let mut attempt = 0u32;
    let (error, retried) = loop {
        match ncm().await {
            Ok(input) => return Ok(input),
            Err(e) if should_retry_ncm(&e, attempt) => {
                log::warn!(
                    "NCM解析失败，重试 {}/2: {} - {}: {}",
                    attempt + 1,
                    song.name,
                    song.singer,
                    e
                );
                attempt += 1;
            }
            Err(e) => break (e, attempt > 0),
        }
    };

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    log::info!(
        "[HEAP] after resolve_ncm FAIL (id={}): {} kB — {}",
        song.id,
        mem_rss_kb(),
        error
    );
    if retried {
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        log::info!(
            "[HEAP] after resolve_ncm retries exhausted (id={}): {} kB",
            song.id,
            mem_rss_kb()
        );
        log::warn!(
            "NCM网络错误，2次重试失败，改用云盘兜底: {} - {}",
            song.name,
            song.singer
        );
    }

    log::info!(
        "NCM解析失败，尝试云盘兜底: {} - {} ({})",
        song.name,
        song.singer,
        error
    );

    // The cloud disk is the last resort, and everything it cannot do is silent: the NCM failure
    // stays the reported error.
    cloud().await.ok_or(error)
}

/// Buffer state shared with the stream download progress callback, used to judge whether
/// enough has been buffered before starting playback.
#[derive(Clone)]
struct StreamProgress {
    /// Number of bytes downloaded (written to storage) so far; see `StreamState.current_position`.
    buffered: Arc<AtomicU64>,
    /// Whether the whole stream has finished downloading.
    completed: Arc<AtomicBool>,
}

/// Resolves audio inputs for songs via local files, NCM streaming, or the user's own cloud disk.
#[derive(Clone)]
pub struct AudioSource {
    service: ApiService,
    pub cache: Arc<CacheManager>,
    quality: SongQuality,
    /// Save while playing: when true, stream and write the file into the download cache; when
    /// false, stream to a temporary file only.
    save_on_play: bool,
    /// HTTP client used for streaming play URLs (proxy + headers applied).
    stream_client: reqwest::Client,
    event_tx: mpsc::UnboundedSender<Event>,
}

impl AudioSource {
    pub(super) fn new(
        service: ApiService,
        cache: Arc<CacheManager>,
        quality: SongQuality,
        save_on_play: bool,
        stream_client: reqwest::Client,
        event_tx: mpsc::UnboundedSender<Event>,
    ) -> Self {
        Self {
            service,
            cache,
            quality,
            save_on_play,
            stream_client,
            event_tx,
        }
    }

    /// Toggle save-on-play at runtime: when true, stream and write the file into the download
    /// cache; when false, stream to a temporary file only.
    pub(super) fn set_save_on_play(&mut self, enabled: bool) {
        self.save_on_play = enabled;
    }

    /// Change the stream quality used by the next resolve. The current track
    /// keeps playing; the new level applies when playback moves on.
    pub(super) fn set_quality(&mut self, quality: ncm_api::SongQuality) {
        self.quality = quality;
    }

    /// Build stream-download settings that persist the cache entry and notify
    /// the UI once the stream has finished caching to disk. The entry is only
    /// recorded when the download actually completes, so the index never
    /// contains partial files — those caused cache misses (and a re-download)
    /// on restart.
    ///
    /// Besides the cache bookkeeping it also tracks the download progress so
    /// [`Self::wait_for_prebuffer`] can judge whether enough of the stream is
    /// buffered before playback starts. When `mark_cache` is `false` (streaming to a
    /// temporary file with save-on-play off) the progress is still tracked but nothing is
    /// recorded in the cache index.
    fn tracked_settings(
        &self,
        mark_cache: bool,
        song: &SongInfo,
        ext: &'static str,
    ) -> (Settings<HttpStream<HeadersClient>>, StreamProgress) {
        let event_tx = self.event_tx.clone();
        let cache = self.cache.clone();
        let sent = Arc::new(AtomicBool::new(false));
        let song = song.clone();
        let progress = StreamProgress {
            buffered: Arc::new(AtomicU64::new(0)),
            completed: Arc::new(AtomicBool::new(false)),
        };
        let buffered = progress.buffered.clone();
        let completed = progress.completed.clone();
        let settings = Settings::default().on_progress(move |_, state, _| {
            buffered.store(state.current_position, Ordering::SeqCst);
            let complete = state.phase == StreamPhase::Complete;
            if complete {
                completed.store(true, Ordering::SeqCst);
            }
            // Record the cache entry only once the download has finished: an entry written
            // mid-download would list a truncated file as playable and would let eviction
            // delete a file that is still being written.
            if should_record_cache(mark_cache, complete, &sent) {
                cache.mark_cached(&song, ext);
                let _ = event_tx.send(PlaybackEvent::Cached(song.id).into());
            }
        });
        (settings, progress)
    }

    /// Wait until the stream download has buffered [`PREBUFFER_BYTES`] bytes (or finished
    /// downloading) before returning, giving playback a head buffer so a slow stream download
    /// doesn't underrun at the start. If [`PREBUFFER_TIMEOUT`] is exceeded without reaching the
    /// target (slow network or a broken stream), also return as usual — better to start playing
    /// with occasional stutter than to block playback indefinitely.
    async fn wait_for_prebuffer(&self, progress: &StreamProgress) {
        let deadline = tokio::time::Instant::now() + PREBUFFER_TIMEOUT;
        loop {
            if progress.completed.load(Ordering::SeqCst)
                || progress.buffered.load(Ordering::SeqCst) >= PREBUFFER_BYTES
            {
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                log::warn!(
                    "预缓冲超时（{}），仅缓冲 {} 字节，开始播放",
                    PREBUFFER_TIMEOUT.as_secs_f32(),
                    progress.buffered.load(Ordering::SeqCst)
                );
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Stream `url` through the cache layer. When "边听边存" is enabled the file is
    /// persisted to the download cache (and indexed on completion); otherwise it is
    /// streamed to a temporary file that is cleaned up when playback ends.
    async fn build_stream(
        &self,
        url: url::Url,
        song: &SongInfo,
        ext: &'static str,
    ) -> Result<AudioInput, SourceError> {
        let stream = HttpStream::new(HeadersClient::new(self.stream_client.clone()), url)
            .await
            .map_err(|e| SourceError::Stream(format!("流初始化失败: {e}")))?;

        let (settings, progress) = self.tracked_settings(self.save_on_play, song, ext);

        let reader: Box<dyn AudioReader> = if self.save_on_play {
            let provider = self
                .cache
                .create_provider(song, ext)
                .map_err(|e| SourceError::Stream(format!("缓存创建失败: {e}")))?;
            Box::new(
                StreamDownload::from_stream(stream, provider, settings)
                    .await
                    .map_err(|e| SourceError::Stream(format!("流下载失败: {e}")))?,
            )
        } else {
            Box::new(
                StreamDownload::from_stream(stream, TempStorageProvider::default(), settings)
                    .await
                    .map_err(|e| SourceError::Stream(format!("流下载失败: {e}")))?,
            )
        };

        self.wait_for_prebuffer(&progress).await;

        Ok(SharedReader(Arc::new(Mutex::new(reader))))
    }

    /// Derive a file extension from a streaming URL.
    fn ext_from_url(url: &str) -> &'static str {
        let path = url::Url::parse(url)
            .ok()
            .and_then(|u| {
                u.path_segments()
                    .and_then(|mut s| s.next_back().map(|s| s.to_string()))
            })
            .unwrap_or_default();
        let stem = path.rsplit('.').nth(1).unwrap_or("");
        match stem {
            "flac" => "flac",
            "ogg" => "ogg",
            "wav" => "wav",
            "m4a" | "mp4" => "m4a",
            _ => "mp3",
        }
    }

    /// Try to resolve a song from NCM streaming.
    async fn resolve_ncm(&self, song: &SongInfo) -> Result<AudioInput, SourceError> {
        self.resolve_ncm_id(song, song.id).await
    }

    /// Try to resolve a song from NCM streaming by fetching the player URL of `id`.
    ///
    /// `id` is the song's catalogue id for [`Self::resolve_ncm`] and the id of the matching
    /// cloud-disk entry for the cloud fallback: a cloud song is served by the same player-URL API,
    /// so both paths share the URL request, the empty/trial-URL guard and the stream setup. `song`
    /// stays the song being played either way — it owns the cache key and the events.
    async fn resolve_ncm_id(&self, song: &SongInfo, id: u64) -> Result<AudioInput, SourceError> {
        let urls = self.service.fetch_song_urls(&[id], self.quality).await;

        let urls = match urls {
            Ok(u) => u,
            Err(NcmError::Http(e)) => {
                return Err(SourceError::Network(format!("获取歌曲URL失败: {e}")));
            }
            Err(NcmError::Session(e)) => {
                return Err(SourceError::Network(format!("会话异常: {e}")));
            }
            Err(e) => {
                return Err(SourceError::Unavailable(format!("获取歌曲URL失败: {e}")));
            }
        };

        let url_str = urls
            .iter()
            .find(|u| !u.url.is_empty() && !u.free_trial)
            .map(|u| &u.url)
            .ok_or_else(|| {
                SourceError::Unavailable("该歌曲暂无播放源（可能需要 VIP 或版权受限）".to_string())
            })?;

        let url = url::Url::parse(url_str)
            .map_err(|e| SourceError::Stream(format!("URL解析失败: {e}")))?;
        let ext = Self::ext_from_url(url_str);

        self.build_stream(url, song, ext).await
    }

    /// Last resort before playback is declared failed: find the song in the user's own cloud disk
    /// and stream the uploaded copy. A track NCM refuses to hand out a URL for (VIP-only,
    /// copyright, region) is still playable when the user uploaded it themselves.
    ///
    /// Everything that cannot produce a stream is silent and returns `None` — no login, no match, a
    /// failing cloud API, an unusable cloud URL — so the caller keeps its original error: playback
    /// was asked for, and a second, vaguer failure message ("云盘里没有这首歌") would replace a
    /// precise one ("需要 VIP"). The disk is paged from the start because the endpoint only offsets
    /// into a list and offers no search; the scan stops at [`CLOUD_DISK_SCAN_LIMIT`] entries.
    async fn resolve_cloud(&self, song: &SongInfo) -> Option<AudioInput> {
        let mut offset = 0u32;
        while offset < CLOUD_DISK_SCAN_LIMIT {
            let page = match self
                .service
                .client()
                .user_cloud_disk(offset, CLOUD_DISK_PAGE)
                .await
            {
                Ok(page) => page,
                Err(e) => {
                    // Not logged in, or the cloud API is unhappy: the fallback is optional, so the
                    // original failure is reported instead.
                    log::info!("云盘兜底跳过：获取云盘列表失败 (offset={offset}): {e}");
                    return None;
                }
            };

            if let Some(id) = pick_cloud_match(song, &page.songs) {
                log::info!(
                    "云盘兜底命中: {} - {} (songId={id}, offset={offset})",
                    song.name,
                    song.singer
                );
                return match self.resolve_ncm_id(song, id).await {
                    Ok(input) => Some(input),
                    Err(e) => {
                        log::info!("云盘地址解析失败，放弃云盘兜底: {e}");
                        None
                    }
                };
            }

            if !page.has_more || page.songs.is_empty() {
                break;
            }
            offset += CLOUD_DISK_PAGE;
        }

        log::info!("云盘兜底未命中: {} - {}", song.name, song.singer);
        None
    }

    /// Return a previously cached audio file for `song`, if one exists.
    async fn resolve_cached(&self, song: &SongInfo) -> Option<AudioInput> {
        let ext = self.cache.find_cached_extension(song.id)?.to_string();
        let cache = self.cache.clone();
        let song_id = song.id;
        let file = tokio::task::spawn_blocking(move || cache.open_cached(song_id, &ext))
            .await
            .ok()?
            .ok()?;
        Some(SharedReader(Arc::new(Mutex::new(Box::new(file)))))
    }

    /// Open a local file for a `Free` song; see [`local_file_path`] for which field holds it.
    async fn resolve_local(&self, song: &SongInfo) -> Option<AudioInput> {
        if song.copyright != ncm_api::SongCopyright::Free {
            return None;
        }
        let path = std::path::PathBuf::from(local_file_path(song));
        if !path.exists() {
            return None;
        }
        let file = tokio::task::spawn_blocking(move || std::fs::File::open(path))
            .await
            .ok()?
            .ok()?;
        Some(SharedReader(Arc::new(Mutex::new(Box::new(file)))))
    }

    pub(super) async fn resolve(&self, song: &SongInfo) -> Result<AudioInput, SourceError> {
        // 1. Cache wins for every source.
        if let Some(input) = self.resolve_cached(song).await {
            return Ok(input);
        }

        // 2. Free NCM songs may point at a local file path.
        if let Some(input) = self.resolve_local(song).await {
            return Ok(input);
        }

        // 3. NCM streaming, then the user's own cloud disk: the song is only declared
        //    unplayable when both failed.
        resolve_streaming(song, || self.resolve_ncm(song), || self.resolve_cloud(song)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, sync::atomic::AtomicU32};

    /// The catalog song under test: VIP-only (so NCM is the one that fails on it), four minutes of
    /// audio at the durations uploads are usually tagged with.
    fn test_song() -> SongInfo {
        SongInfo {
            id: 42,
            name: "Hello, World!".into(),
            singer: "A/B".into(),
            artist_id: 1,
            album: "An Album".into(),
            album_id: 2,
            pic_url: String::new(),
            duration: 240_000,
            mv: 0,
            copyright: ncm_api::SongCopyright::VipOnly,
            local_path: None,
        }
    }

    /// The NCM retry/fallback policy: a transport failure gets exactly one retry, while
    /// an unplayable song falls through to the cloud-disk fallback immediately (retrying
    /// cannot turn a VIP-only track into a playable one). This used to be encoded in a
    /// `"NETWORK:"` string prefix.
    #[test]
    fn only_network_failures_are_retried() {
        assert!(
            should_retry_ncm(&SourceError::Network("boom".into()), 0),
            "a transport failure must be retried once"
        );
        assert!(
            !should_retry_ncm(&SourceError::Network("boom".into()), 1),
            "the retry must happen only once"
        );
        assert!(
            !should_retry_ncm(&SourceError::Unavailable("暂无播放源".into()), 0),
            "a song with no playable URL must fall back immediately"
        );
        assert!(
            !should_retry_ncm(&SourceError::Stream("decode".into()), 0),
            "a stream setup failure must fall back immediately"
        );
    }

    /// A download-cache entry is recorded on completion only, and only once per stream:
    /// recording mid-download would list a truncated file as playable, and skipping the
    /// entry would leave a completed file unmanaged on disk.
    #[test]
    fn cache_entry_is_recorded_only_after_completion() {
        let sent = AtomicBool::new(false);

        assert!(
            !should_record_cache(true, false, &sent),
            "mid-download must not write an index entry"
        );
        assert!(
            !should_record_cache(false, true, &sent),
            "save_on_play = false must not write an index entry"
        );
        assert!(
            should_record_cache(true, true, &sent),
            "a completed download must be recorded"
        );
        assert!(
            !should_record_cache(true, true, &sent),
            "a repeated completion callback must not record twice"
        );
    }

    /// A local song plays from `local_path`. `album` is only the fallback for songs that still
    /// carry their path there: it stopped being the path once a local file gained a real album
    /// tag, and reading it there sends tagged files to the network resolver even though the
    /// file is on disk.
    #[test]
    fn local_songs_prefer_the_path_over_the_album() {
        let mut song = test_song();

        song.local_path = Some("/music/local.flac".into());
        song.album = "An Album".into();
        assert_eq!(local_file_path(&song), "/music/local.flac");

        song.local_path = None;
        song.album = "/music/legacy.mp3".into();
        assert_eq!(local_file_path(&song), "/music/legacy.mp3");
    }

    /// The cloud-disk match has to survive hand-tagged uploads: titles and artists are compared
    /// case- and punctuation-insensitively, an artist that appears on either side is enough, and a
    /// duration NCM does not know (`0`) must not disqualify the entry — while a title that is a
    /// different recording by another artist, or a different length, must not be played.
    #[test]
    fn cloud_match_tolerates_upload_tags() {
        let song = test_song();
        let upload = |id: u64, name: &str, singer: &str, duration: u64| SongInfo {
            id,
            name: name.to_string(),
            singer: singer.to_string(),
            duration,
            ..test_song()
        };

        assert_eq!(
            pick_cloud_match(
                &song,
                &[upload(7, "Hello, World! (Remastered 2011)", "a/B", 0)]
            ),
            Some(7),
            "spacing, case, a tag suffix and an unknown duration must still match"
        );
        assert_eq!(
            pick_cloud_match(&song, &[upload(7, "Hello, World!", "someone else", 0)]),
            None,
            "an upload of another artist must not be played as this song"
        );
        assert_eq!(
            pick_cloud_match(&song, &[upload(7, "Hello, World! Tonight", "A", 240_000)]),
            None,
            "a title that merely starts the same is a different song"
        );
        assert_eq!(
            pick_cloud_match(
                &song,
                &[
                    upload(7, "Hello, World!", "A", 0),
                    upload(9, "hello world", "A", 240_500),
                ]
            ),
            Some(9),
            "with two uploads, the one whose duration fits the catalogue entry wins"
        );
        assert_eq!(
            pick_cloud_match(&song, &[upload(7, "Hello, World!", "A", 60_000)]),
            None,
            "a minute-long recording is not this song"
        );
    }

    /// An in-memory stand-in for a resolved stream, carrying the bytes of the step that produced
    /// it: the real inputs have one type, so a test cannot tell them apart otherwise.
    fn fake_input(marker: &[u8]) -> AudioInput {
        SharedReader(Arc::new(Mutex::new(Box::new(std::io::Cursor::new(
            marker.to_vec(),
        )))))
    }

    /// The marker bytes of a [`fake_input`].
    fn marker_of(input: &AudioInput) -> Vec<u8> {
        let mut marker = Vec::new();
        input
            .0
            .lock()
            .expect("the fake reader is not poisoned")
            .read_to_end(&mut marker)
            .expect("an in-memory reader cannot fail");
        marker
    }

    /// The streaming chain with fake resolvers, because neither can be exercised offline: the
    /// cloud disk is the last resort, so it must not be asked when NCM streaming already worked
    /// (that would be a request on the common path), it must hand over its URL when NCM failed,
    /// and when neither can play the song the caller must see the NCM failure.
    #[tokio::test]
    async fn streaming_chain_consults_the_cloud_disk_last() {
        let song = test_song();

        // (a) NCM streaming works: the cloud disk is not touched.
        let cloud_asked = AtomicBool::new(false);
        let input = resolve_streaming(
            &song,
            || async { Ok(fake_input(b"ncm")) },
            || async {
                cloud_asked.store(true, Ordering::SeqCst);
                Some(fake_input(b"cloud"))
            },
        )
        .await
        .expect("NCM streaming succeeded");
        assert_eq!(marker_of(&input), b"ncm");
        assert!(
            !cloud_asked.load(Ordering::SeqCst),
            "普通解析成功时不该访问云盘"
        );

        // (b) NCM fails, the cloud disk holds the song: the uploaded copy is played.
        let input = resolve_streaming(
            &song,
            || async { Err(SourceError::Unavailable("该歌曲暂无播放源".into())) },
            || async { Some(fake_input(b"cloud")) },
        )
        .await
        .expect("the cloud disk matched");
        assert_eq!(marker_of(&input), b"cloud");

        // (c) the cloud disk cannot play it either: the NCM failure is what the caller sees.
        let error = resolve_streaming(
            &song,
            || async { Err(SourceError::Unavailable("该歌曲暂无播放源".into())) },
            || async { None },
        )
        .await
        .expect_err("nothing can play the song");
        assert!(
            matches!(&error, SourceError::Unavailable(message) if message == "该歌曲暂无播放源"),
            "云盘未命中时必须返回 NCM 的错误, got {error:?}"
        );

        // (d) the retry policy still runs in front of the fallback: a transport failure is
        // retried once, and the cloud disk is asked after that second attempt, not instead of it.
        let attempts = AtomicU32::new(0);
        let cloud_asked = AtomicBool::new(false);
        let error = resolve_streaming(
            &song,
            || async {
                attempts.fetch_add(1, Ordering::SeqCst);
                Err(SourceError::Network("boom".into()))
            },
            || async {
                cloud_asked.store(true, Ordering::SeqCst);
                None
            },
        )
        .await
        .expect_err("the transport failure is reported");
        assert_eq!(attempts.load(Ordering::SeqCst), 2, "网络错误只重试一次");
        assert!(cloud_asked.load(Ordering::SeqCst), "重试耗尽后仍要尝试云盘");
        assert!(
            matches!(&error, SourceError::Network(message) if message == "boom"),
            "重试耗尽且云盘未命中时返回网络错误, got {error:?}"
        );
    }
}
