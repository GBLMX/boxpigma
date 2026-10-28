//! `boxpigma update`: in-place upgrade and rollback for an installed boxpigma.
//!
//! This is the versioned layout and the atomic switch of `current` that `install.sh` already
//! implements, redone inside the process: upgrading no longer needs a shell, curl or tar. The
//! script keeps its own job — first installs (creating the directory, the PATH entry) and
//! script-side rollback. `update` only works on a directory the script already installed: no
//! `releases/` means there is nothing to update, and it says so instead of scaffolding a fresh
//! install.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use color_eyre::eyre::{Result, bail, eyre};
use reqwest::Client;
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::FormatItem, macros::format_description};
use url::Url;

use crate::config::UpdateChannel;

/// Where releases come from; `--mirror` replaces the `https://github.com` prefix.
const REPO: &str = "GBLMX/pigma";
const HOST: &str = "https://github.com";
/// Canonical repository after the pigma → boxpigma rename. Older manifests still say
/// `GBLMX/pigma`; GitHub redirects, but proxy mirrors handle the redirect poorly.
const CANONICAL_REPO: &str = "GBLMX/boxpigma";
/// Stable manifest used when no channel-specific feed is available.
const FALLBACK_MANIFEST_URL: &str =
    "https://github.com/GBLMX/pigma/releases/latest/download/boxpigma-updates.json";
/// How many version directories to keep, current included — install.sh's `$Keep`.
const KEEP: usize = 3;
/// The release target this binary was built for. `build.rs` bakes in cargo's own `TARGET` so
/// the asset name (`boxpigma-<target>.tar.gz`) matches the release even when the build was
/// cross-compiled.
const TARGET: &str = env!("BOXPIGMA_TARGET");
const BIN: &str = "boxpigma";

/// `install.lock`'s timestamp format, install.sh's `date -u '+%Y-%m-%dT%H:%M:%SZ'`.
const INSTALLED_AT_FMT: &[FormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

/// Everything `boxpigma update` accepts, filled in by `cli.rs` from the command line.
pub struct Options {
    pub check: bool,
    pub version: String,
    pub dir: Option<PathBuf>,
    pub mirror: Option<String>,
    pub checksums: Option<String>,
    pub force: bool,
    /// Install even when `SHA256SUMS` cannot be fetched, i.e. without an integrity check.
    pub insecure: bool,
    pub dry_run: bool,
    pub rollback: bool,
    /// Release feed selected by `[update] channel`, `--channel`, or the environment.
    pub channel: UpdateChannel,
    /// Full manifest URL override (`BOXPIGMA_UPDATE_URL` or `--manifest`).
    pub manifest: Option<String>,
}

/// One release feed document. It intentionally has the same shape as the launcher's
/// `ferriso-updates.json`: version, notes, date, and a platform map.
#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    pub_date: Option<String>,
    #[serde(default)]
    platforms: HashMap<String, Platform>,
}

#[derive(Debug, Deserialize)]
struct Platform {
    url: String,
    #[serde(default)]
    install_urls: Vec<String>,
    #[serde(default)]
    signature: Option<String>,
    /// Boxpigma-specific integrity value. Older manifests may omit it; the updater then
    /// falls back to the release's sibling `SHA256SUMS`.
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Clone)]
struct ReleaseInfo {
    version: String,
    notes: Option<String>,
    published: Option<String>,
    asset_url: String,
    install_url: String,
    signature: Option<String>,
    sha256: Option<String>,
}

/// Run `boxpigma update` end to end.
///
/// The order is the behaviour: resolve the directory, refuse anything that is not an install,
/// then answer `--check` / `--rollback` / install in that order. Nothing here creates the
/// install directory or touches `PATH` — that is install.sh's job.
pub async fn run(opts: Options) -> Result<()> {
    let layout = Layout {
        dir: resolve_dir(opts.dir.as_deref())?,
    };

    // The same three preconditions the install script checks, in the same order.
    if layout.dir.exists() && !layout.dir.is_dir() {
        bail!("{} 已经存在，而且不是目录", layout.dir.display());
    }
    let current = layout.current();
    if fs::symlink_metadata(&current).is_ok() && fs::read_link(&current).is_err() {
        bail!(
            "{} 已经存在，而且不是目录联接或符号链接 —— 请先删掉它，或换一个 --dir",
            current.display()
        );
    }
    let releases = layout.releases();
    if !releases.is_dir() {
        bail!(
            "没有看到安装：{} 不存在 —— 首次安装请用 install.sh",
            releases.display()
        );
    }

    let client = http_client()?;

    if opts.check {
        check(&client, &layout, &opts).await?;
        return Ok(());
    }
    if opts.rollback {
        return rollback(&layout, opts.dry_run);
    }

    install(&client, &layout, &opts).await
}

/// The install root: `--dir` verbatim (a relative path stays relative to the CWD — no
/// `canonicalize`, so the path is used exactly as given), else the root the running executable
/// belongs to, else the default `~/.local/bin`.
fn resolve_dir(given: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = given {
        return Ok(dir.to_path_buf());
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = install_dir_from_exe(&exe)
    {
        return Ok(dir);
    }
    if let Some(dir) = default_install_dir() {
        return Ok(dir);
    }
    bail!("拿不到安装目录 —— 用 --dir 指定")
}

/// The install root `exe` lives in, or `None` when it lives outside one.
///
/// The path as given and its canonicalized form are both tried: `current` is a symlink, so a
/// canonicalized `<dir>/current/boxpigma` resolves to `<dir>/releases/<rel>/boxpigma`, and
/// both spellings have to be recognized. No canonicalization is needed for the common case,
/// which is why it comes second.
fn install_dir_from_exe(exe: &Path) -> Option<PathBuf> {
    let canonical = fs::canonicalize(exe).unwrap_or_else(|_| exe.to_path_buf());
    for root in [exe.to_path_buf(), canonical] {
        let Some(dir) = root.parent() else { continue };
        // `<dir>/current/boxpigma`
        if dir.file_name().is_some_and(|name| name == "current") {
            return dir.parent().map(Path::to_path_buf);
        }
        // `<dir>/releases/<rel>/boxpigma`
        if dir
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "releases")
        {
            return dir.parent().and_then(Path::parent).map(Path::to_path_buf);
        }
        // The layout before `releases/` existed: `<dir>/boxpigma`.
        if dir.join("releases").is_dir() {
            return Some(dir.to_path_buf());
        }
    }
    None
}

/// Where the install script puts a fresh install.
fn default_install_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".local").join("bin"))
}

/// One client for every request: the 300 s total timeout is install.sh's `curl --max-time 300`
/// (the checksum request shares the ceiling, though it moves a few hundred bytes). Proxy
/// mirrors used by `--mirror` are more reliable over HTTP/1.1 than HTTP/2. The TLS
/// provider is installed once at startup in `main.rs`.
fn http_client() -> Result<Client> {
    Client::builder()
        .user_agent("boxpigma-update")
        .http1_only()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(Into::into)
}

/// `--mirror` when given, `https://github.com` otherwise.
fn host_of(mirror: Option<&str>) -> &str {
    mirror.filter(|host| !host.is_empty()).unwrap_or(HOST)
}

/// Apply `--mirror` to a GitHub URL, not just to URLs built from `HOST`.
fn mirror_url(url: &str, mirror: Option<&str>) -> String {
    let Some(host) = mirror.filter(|host| !host.is_empty()) else {
        return url.to_owned();
    };
    let legacy = format!("{HOST}/{REPO}");
    let canonical = format!("{HOST}/{CANONICAL_REPO}");
    let url = url
        .strip_prefix(&legacy)
        .map(|rest| format!("{canonical}{rest}"))
        .unwrap_or_else(|| url.to_owned());
    url.strip_prefix(HOST)
        .map(|rest| format!("{host}{rest}"))
        .unwrap_or(url)
}

/// Where this release's assets live, spelled the way install.sh spells it.
fn base_url(host: &str, version: &str) -> String {
    if version == "latest" {
        format!("{host}/{REPO}/releases/latest/download")
    } else {
        format!("{host}/{REPO}/releases/download/{version}")
    }
}

/// The release archive for this platform.
fn asset_name() -> String {
    format!("boxpigma-{TARGET}.tar.gz")
}

/// The platform key used by the native update manifests.
fn platform_key() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn explicit_manifest(opts: &Options) -> Option<String> {
    opts.manifest
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("BOXPIGMA_UPDATE_URL")
                .ok()
                .map(|url| url.trim().to_string())
                .filter(|url| !url.is_empty())
        })
}

async fn fetch_manifest(client: &Client, url: &str) -> Result<Manifest> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| eyre!("拿不到更新清单 {url}：{error}"))?;
    if !response.status().is_success() {
        bail!("拿不到更新清单 {url}：HTTP {}", response.status());
    }
    response
        .json::<Manifest>()
        .await
        .map_err(|error| eyre!("更新清单 {url} 格式不对：{error}"))
}

/// Fetch the channel manifest, falling back to the stable GitHub asset. The fallback
/// mirrors the launcher: a mirror outage must not make an installed client unable to
/// discover a release.
async fn fetch_release(client: &Client, opts: &Options) -> Result<ReleaseInfo> {
    let explicit = explicit_manifest(opts);
    let primary = explicit
        .clone()
        .unwrap_or_else(|| opts.channel.manifest_url().to_string());
    let manifest = match fetch_manifest(client, &primary).await {
        Ok(manifest) => manifest,
        Err(primary_error) if explicit.is_none() => fetch_manifest(client, FALLBACK_MANIFEST_URL)
            .await
            .map_err(|fallback_error| eyre!("{primary_error}; {fallback_error}"))?,
        Err(error) => return Err(error),
    };

    let mut release = release_from_manifest(manifest, &platform_key())?;
    release.asset_url = mirror_url(&release.asset_url, opts.mirror.as_deref());
    release.install_url = mirror_url(&release.install_url, opts.mirror.as_deref());
    Ok(release)
}

fn release_from_manifest(manifest: Manifest, key: &str) -> Result<ReleaseInfo> {
    let version = normalized_version(&manifest.version)?;
    let platform = manifest
        .platforms
        .get(key)
        .ok_or_else(|| eyre!("更新清单里没有 {key} 的产物"))?;
    asset_from_url(&platform.url)?;
    let install_url = platform
        .install_urls
        .first()
        .cloned()
        .unwrap_or_else(|| platform.url.clone());
    Ok(ReleaseInfo {
        version,
        notes: manifest.notes,
        published: manifest.pub_date,
        asset_url: platform.url.clone(),
        install_url,
        signature: platform.signature.clone(),
        sha256: platform.sha256.clone(),
    })
}

fn normalized_version(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let raw = raw.strip_prefix('v').unwrap_or(raw);
    let version =
        Version::parse(raw).map_err(|error| eyre!("更新清单里的版本 `{raw}` 不合法：{error}"))?;
    Ok(version.to_string())
}

/// Take only the final path segment of an asset URL, and reject anything that could
/// become a path component outside the install directory.
fn asset_from_url(url: &str) -> Result<String> {
    let parsed = Url::parse(url).map_err(|error| eyre!("资产 URL `{url}` 不合法：{error}"))?;
    let name = parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .filter(|name| {
            !name.is_empty()
                && *name != "."
                && *name != ".."
                && !name.contains('/')
                && !name.contains('\\')
        })
        .ok_or_else(|| eyre!("资产 URL `{url}` 没有安全的文件名"))?;
    Ok(name.to_string())
}

fn is_newer(candidate: &str, current: &str) -> bool {
    let Ok(candidate) = Version::parse(candidate.trim_start_matches('v')) else {
        return false;
    };
    let Ok(current) = Version::parse(current.trim_start_matches('v')) else {
        return false;
    };
    candidate > current
}

fn checksums_url(asset_url: &str) -> String {
    let prefix = asset_url
        .rsplit_once('/')
        .map_or(asset_url, |(prefix, _)| prefix);
    format!("{prefix}/SHA256SUMS")
}

/// The version number a pinned `--version` names, or `None` for `latest` (which the binary
/// itself reports only after unpacking).
fn explicit_version(version: &str) -> Result<Option<String>> {
    if version == "latest" {
        return Ok(None);
    }
    normalized_version(version)
        .map(Some)
        .map_err(|error| eyre!("版本号看着不对：{version}（{error}）"))
}

/// `1.6.0-x86_64-unknown-linux-gnu` → `1.6.0`, the script's `${MATCH%-$TARGET}`.
fn version_number_from_dir(name: &str) -> String {
    name.strip_suffix(&format!("-{TARGET}"))
        .unwrap_or(name)
        .to_string()
}

/// Lowercase hex of the SHA-256 of `bytes` — the format `SHA256SUMS` uses.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// This asset's hash in a `SHA256SUMS` body, one `<hash>  <name>` record per line.
fn parse_checksums(body: &str, asset: &str) -> Option<String> {
    for line in body.lines() {
        let mut fields = line.split_whitespace();
        let (Some(hash), Some(name)) = (fields.next(), fields.next()) else {
            continue;
        };
        if name.to_lowercase() == asset.to_lowercase() {
            return Some(hash.to_lowercase());
        }
    }
    None
}

/// Run a binary and pull the version out of its `--version` output: `boxpigma 1.6.0` →
/// `1.6.0`. That is the first run of digits followed by version-number characters, which is
/// what install.sh's `sed` / regex extracts.
fn version_of(bin: &Path) -> Option<String> {
    let output = Command::new(bin).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let bytes = text.as_bytes();
    let start = bytes.iter().position(u8::is_ascii_digit)?;
    let end = bytes[start..]
        .iter()
        .position(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-')))
        .map_or(bytes.len(), |offset| start + offset);
    Some(text[start..end].to_string())
}

/// The hash published for this platform's asset, or `None` when the list could not be fetched
/// at all (no network, or a release old enough to predate `SHA256SUMS`).
///
/// `source` is a local file when one exists at that path and a URL otherwise — the same
/// `<url|file>` install.sh accepts.
async fn fetch_checksums(client: &Client, source: &str, asset: &str) -> Result<Option<String>> {
    let body = if Path::new(source).is_file() {
        fs::read_to_string(source).unwrap_or_default()
    } else {
        match client.get(source).send().await {
            Ok(response) if response.status().is_success() => {
                response.text().await.unwrap_or_default()
            }
            _ => return Ok(None),
        }
    };
    if body.trim().is_empty() {
        return Ok(None);
    }
    match parse_checksums(&body, asset) {
        Some(want) => Ok(Some(want)),
        // The list arrived but does not mention this asset — not something to install past.
        None => bail!("SHA256SUMS 里没有 {asset} 的记录（{source}）"),
    }
}

/// The already-installed version directory this release matches, if any: same recorded hash as
/// the published one, or — when there is no hash to compare — the same version number.
fn reused(layout: &Layout, want: Option<&str>, pinned: Option<&str>) -> Option<String> {
    if let Some(want) = want {
        let candidates = match pinned {
            Some(number) => vec![format!("{number}-{TARGET}")],
            // `--version latest`: whichever directory was downloaded from this same release.
            None => layout.ordered_dirs(),
        };
        for name in candidates {
            let dir = layout.releases().join(&name);
            if !dir.join(BIN).is_file() {
                continue;
            }
            let Ok(recorded) = fs::read_to_string(dir.join(".archive.sha256")) else {
                continue;
            };
            if recorded.trim() == want {
                return Some(name);
            }
        }
    }

    // No checksum to compare against (a release older than `SHA256SUMS`): ask the binary.
    let number = pinned?;
    let candidate = format!("{number}-{TARGET}");
    let bin = layout.releases().join(&candidate).join(BIN);
    if bin.is_file() && version_of(&bin).as_deref() == Some(number) {
        return Some(candidate);
    }
    None
}

/// Download the release archive, mapping onto install.sh's messages the failures it tells
/// apart: 404 (no asset for this platform, or a wrong tag), a transport error, and anything
/// else that came back with a status code.
async fn download(client: &Client, url: &str, version: &str, asset: &str) -> Result<Vec<u8>> {
    let response =
        client.get(url).send().await.map_err(|error| {
            eyre!("下载失败：{url} —— {error}；网络不通？可以用 --mirror 指向镜像")
        })?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        bail!(
            "release {version} 里没有 {asset}（{url}）—— 这个平台可能没有发布产物，或 tag 写错了"
        );
    }
    if !status.is_success() {
        bail!("下载失败：{url}（HTTP {}）", status.as_u16());
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| eyre!("下载失败：{url} —— {error}；网络不通？可以用 --mirror 指向镜像"))
}

struct ReleaseSpec {
    label: String,
    asset: String,
    asset_url: String,
    checksums_source: String,
    pinned: Option<String>,
    manifest_sha256: Option<String>,
}

/// Resolve `latest` through the channel manifest and fall back to the release redirect
/// when no manifest exists yet. A pinned `--version` stays on the direct GitHub/mirror path.
async fn resolve_release(client: &Client, opts: &Options) -> Result<ReleaseSpec> {
    if opts.version == "latest" {
        match fetch_release(client, opts).await {
            Ok(release) => {
                let asset = asset_from_url(&release.asset_url)?;
                let checksums_source = match opts.checksums.as_deref() {
                    Some(source) if !source.is_empty() => source.to_string(),
                    _ => checksums_url(&release.asset_url),
                };
                let manifest_sha256 = opts
                    .checksums
                    .as_deref()
                    .is_none_or(str::is_empty)
                    .then_some(release.sha256)
                    .flatten();
                return Ok(ReleaseSpec {
                    label: release.version.clone(),
                    asset,
                    asset_url: release.asset_url,
                    checksums_source,
                    pinned: Some(release.version),
                    manifest_sha256,
                });
            }
            Err(error) => {
                // Do not hide a manifest problem behind a silent downgrade to the redirect.
                eprintln!("更新清单不可用，回退到 latest release：{error}");
            }
        }
    }

    let base = base_url(host_of(opts.mirror.as_deref()), &opts.version);
    let asset = asset_name();
    let asset_url = format!("{base}/{asset}");
    let pinned = explicit_version(&opts.version)?;
    let checksums_source = match opts.checksums.as_deref() {
        Some(source) if !source.is_empty() => source.to_string(),
        _ => format!("{base}/SHA256SUMS"),
    };
    Ok(ReleaseSpec {
        label: opts.version.clone(),
        asset,
        asset_url,
        checksums_source,
        pinned,
        manifest_sha256: None,
    })
}

/// Download, verify, unpack and publish a release — then point `current` at it.
async fn install(client: &Client, layout: &Layout, opts: &Options) -> Result<()> {
    let spec = resolve_release(client, opts).await?;
    let asset = spec.asset.clone();
    let asset_url = spec.asset_url.clone();
    let pinned = spec.pinned.clone();

    println!("boxpigma update: {REPO} {}", spec.label);
    println!("  安装目录   {}", layout.dir.display());
    println!("  平台       {TARGET}");
    println!("  资产       {asset_url}");
    println!(
        "  安装       {}",
        layout
            .releases()
            .join(format!(
                "{}-{TARGET}",
                pinned.as_deref().unwrap_or("<下载后确定>")
            ))
            .join(BIN)
            .display()
    );
    println!("  current    {}", layout.current().display());
    match opts.checksums.as_deref() {
        Some("") => {}
        Some(source) => println!("  校验       {source}"),
        None if spec.manifest_sha256.is_some() => println!("  校验       清单内 SHA256"),
        None => println!("  校验       {}", spec.checksums_source),
    }

    let want = if let Some(want) = spec.manifest_sha256 {
        Some(want)
    } else {
        match fetch_checksums(client, &spec.checksums_source, &asset).await? {
            Some(want) => Some(want),
            // No list means no verification, and an updater that installs unverified bytes is
            // exactly what a mirror user — the one most likely to be missing this file — must not
            // get by accident. So this is a refusal with two ways out, not a warning: `--insecure`
            // says it out loud, `--checksums` supplies a list of your own, and `--dry-run` (which
            // writes nothing) only reports what a real run would do.
            None if opts.insecure => {
                println!(
                    "  校验       拿不到 {} —— --insecure 已放行，这次不做校验",
                    spec.checksums_source
                );
                None
            }
            None if opts.dry_run => {
                println!(
                    "  校验       拿不到 {} —— 真跑会被拒（要 --checksums 或 --insecure）",
                    spec.checksums_source
                );
                None
            }
            None => bail!(
                "取不到校验和 {} —— 已中止：没有校验就不安装。\n               自己指定一份：--checksums <URL|本地文件>\n               或明确放行：  --insecure（不做完整性校验）",
                spec.checksums_source
            ),
        }
    };

    // The reuse check is read-only — no download, no write — so `--dry-run` reports its verdict
    // too: "this version is already installed" is part of the plan.
    let existing = if opts.force {
        None
    } else {
        reused(layout, want.as_deref(), pinned.as_deref())
    };
    if let Some(existing) = &existing {
        println!(
            "  已存在     {} 校验一致，跳过下载（--force 可强制重装）",
            layout.releases().join(existing).display()
        );
    }

    if opts.dry_run {
        println!("  (dry-run：没有下载，也没有写入)");
        return Ok(());
    }

    if let Some(existing) = existing {
        let number = version_number_from_dir(&existing);
        return layout.finish_install(&existing, &number);
    }

    let scratch = Scratch::new(&layout.releases());
    fs::create_dir_all(&scratch.tmp)
        .map_err(|e| eyre!("创建不了临时目录 {}（{e}）", scratch.tmp.display()))?;

    println!("  下载中…");
    let archive = download(client, &asset_url, &spec.label, &asset).await?;
    if let Some(want) = &want {
        let got = sha256_hex(&archive);
        if *want != got {
            bail!(
                "校验失败：{asset} 的 SHA256 对不上（期望 {want}，实际 {got}）—— 已中止，current 没有被改动"
            );
        }
        println!("  校验       ok（{got}）");
    }
    fs::write(scratch.archive(&asset), &archive)
        .map_err(|e| eyre!("写不了 {}（{e}）", scratch.archive(&asset).display()))?;
    drop(archive);

    // Unpack under `releases/` and only then rename into place: within one filesystem the
    // rename is atomic, so a half-written version directory never becomes visible.
    fs::create_dir_all(&scratch.staging).map_err(|e| {
        eyre!(
            "在 {} 下创建临时目录失败（{e}）",
            layout.releases().display()
        )
    })?;
    extract(&scratch.archive(&asset), &scratch.staging)?;

    let staged_bin = scratch.staging.join(BIN);
    if !staged_bin.is_file() {
        bail!("压缩包里没有 {BIN}");
    }
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&staged_bin, fs::Permissions::from_mode(0o755))
        .map_err(|e| eyre!("改不了 {} 的权限（{e}）", staged_bin.display()))?;
    // Current release archives carry the iced frontend too. Older ones do not; keep the
    // updater working with both.
    let staged_gui = scratch.staging.join(format!("{BIN}-gui"));
    if staged_gui.is_file() {
        fs::set_permissions(&staged_gui, fs::Permissions::from_mode(0o755))
            .map_err(|e| eyre!("改不了 {} 的权限（{e}）", staged_gui.display()))?;
    }

    let number = match &pinned {
        Some(number) => number.clone(),
        None => {
            // `--version latest`: only now is the version known.
            let Some(number) = version_of(&staged_bin) else {
                bail!("{BIN} --version 没有给出可用版本号 —— 请显式指定 --version <tag>");
            };
            println!("  版本       latest = {number}");
            number
        }
    };
    let rel = format!("{number}-{TARGET}");
    let version_dir = layout.releases().join(&rel);

    if let Some(want) = &want {
        // Record the archive's hash so the next run for this same version skips the download.
        let _ = fs::write(scratch.staging.join(".archive.sha256"), format!("{want}\n"));
    }

    if version_dir.exists() {
        let _ = fs::remove_dir_all(&scratch.old);
        if fs::rename(&version_dir, &scratch.old).is_err() {
            bail!("挪不动旧目录 {}", version_dir.display());
        }
    }
    if fs::rename(&scratch.staging, &version_dir).is_err() {
        if scratch.old.is_dir() {
            let _ = fs::rename(&scratch.old, &version_dir);
        }
        bail!("装不进 {}", version_dir.display());
    }
    let _ = fs::remove_dir_all(&scratch.old);

    layout.finish_install(&rel, &number)
}

/// `--check`: report what is installed and whether a newer release exists. Reads only —
/// `current` and the lock file are never touched, and being out of date is not an error.
async fn check(client: &Client, layout: &Layout, opts: &Options) -> Result<()> {
    let rel = layout.current_target();
    let installed = rel
        .as_deref()
        .and_then(|rel| version_of(&layout.releases().join(rel).join(BIN)));
    let current = match (&rel, &installed) {
        (Some(rel), Some(number)) => format!("{number}（releases/{rel}）"),
        (Some(rel), None) => format!("releases/{rel}"),
        (None, _) => "（未安装）".to_string(),
    };
    let host = host_of(opts.mirror.as_deref());

    println!("  安装目录   {}", layout.dir.display());
    println!("  当前       {current}");
    println!("  通道       {}", opts.channel);

    match fetch_release(client, opts).await {
        Ok(release) => {
            println!("  最新       v{}", release.version);
            if let Some(published) = &release.published {
                println!("  发布时间   {published}");
            }
            if let Some(notes) = &release.notes {
                println!("  说明       {notes}");
            }
            println!("  资产       {}", release.asset_url);
            println!("  安装包     {}", release.install_url);
            if release.signature.is_some() {
                println!("  签名       已提供");
            }
            if !is_newer(&release.version, installed.as_deref().unwrap_or("0.0.0")) {
                println!("  已是最新   无需升级");
                return Ok(());
            }
            println!("  有新版     运行 `boxpigma update` 升级");
            Ok(())
        }
        Err(manifest_error) => {
            // Releases published before the manifest existed still work through the old
            // latest-tag redirect. Keep that fallback for them.
            eprintln!("更新清单不可用，回退到 latest release：{manifest_error}");
            let latest = latest_tag(client, host).await?;
            println!("  最新       v{latest}");
            if installed.as_deref() == Some(latest.as_str()) {
                println!("  已是最新   无需升级");
                return Ok(());
            }
            println!("  有新版     运行 `boxpigma update` 升级");
            let asset = asset_name();
            let asset_url = format!("{host}/{REPO}/releases/latest/download/{asset}");
            if let Ok(response) = client.head(&asset_url).send().await
                && response.status() == reqwest::StatusCode::NOT_FOUND
            {
                println!("  注意       release v{latest} 里没有 {asset}");
            }
            Ok(())
        }
    }
}

/// The newest release tag, read off the redirect `…/releases/latest` lands on. `host` is
/// `--mirror` when given and `https://github.com` otherwise.
///
/// Deliberately not `api.github.com`: anonymous API calls are capped at 60/hour, while the
/// redirect is not, and install.sh has always used it.
async fn latest_tag(client: &Client, host: &str) -> Result<String> {
    let url = format!("{host}/{REPO}/releases/latest");
    let response = client
        .head(&url)
        .send()
        .await
        .map_err(|_| eyre!("拿不到最新版本号：{url} —— 网络不通？可以用 --mirror 指向镜像"))?;
    let final_url = response.url();
    let tag = final_url
        .path()
        .contains("/releases/tag/")
        .then(|| final_url.path_segments()?.next_back().map(str::to_owned))
        .flatten();
    match tag {
        Some(tag) => Ok(tag.trim_start_matches('v').to_string()),
        None => bail!("拿不到最新版本号：{final_url} 没有跳到 tag —— 用 --version <tag> 指定"),
    }
}

/// `--rollback`: switch `current` to the newest version that is not the current one, the same
/// rule install.sh uses (not the lock's `previous=`, which can name a deleted directory).
fn rollback(layout: &Layout, dry_run: bool) -> Result<()> {
    let current = layout.current_target();
    let Some(target) = layout.rollback_target() else {
        bail!(
            "没有可回滚的版本：{} 里只有 {}",
            layout.releases().display(),
            current.as_deref().unwrap_or("（空目录）")
        );
    };

    if dry_run {
        println!(
            "  current    {} -> releases/{target}",
            current.as_deref().unwrap_or("（无）")
        );
        println!("  (dry-run：没有改动任何东西)");
        return Ok(());
    }

    if layout.switch_current(&target).is_err() {
        if let Some(current) = &current {
            let _ = layout.restore_current(current);
        }
        bail!(
            "回滚失败：未能把 {} 指向 releases/{target}",
            layout.current().display()
        );
    }
    let number = version_number_from_dir(&target);
    layout.write_lock(&target, &number, current.as_deref().unwrap_or(""))?;

    println!("boxpigma update: 已回滚");
    println!("  版本       {number}");
    println!(
        "  current    {} -> releases/{target}",
        layout.current().display()
    );
    println!("  可执行     {}", layout.current().join(BIN).display());
    println!("  上一个版本 {}", current.as_deref().unwrap_or("（无）"));
    println!(
        "  回滚命令   boxpigma update --dir {} --rollback",
        layout.dir.display()
    );
    Ok(())
}

/// The versioned install layout: `<dir>/releases/<version>-<target>`, `<dir>/current` and
/// `<dir>/install.lock`.
///
/// Every operation takes `&self` — there is no global state, which is what lets the tests run
/// the real thing against a scratch directory.
struct Layout {
    dir: PathBuf,
}

impl Layout {
    fn releases(&self) -> PathBuf {
        self.dir.join("releases")
    }

    fn current(&self) -> PathBuf {
        self.dir.join("current")
    }

    fn lock_file(&self) -> PathBuf {
        self.dir.join("install.lock")
    }

    /// A real version directory under `releases/`, or `None` for a link, a scratch directory
    /// (both start with `.`) or anything that is not a directory.
    ///
    /// `symlink_metadata` is the point: it describes the link itself, so a symlink to a
    /// directory is not mistaken for one.
    fn version_dir(&self, name: &str) -> Option<PathBuf> {
        if name.starts_with('.') {
            return None;
        }
        let path = self.releases().join(name);
        fs::symlink_metadata(&path)
            .is_ok_and(|meta| meta.is_dir())
            .then_some(path)
    }

    /// The directory name `current` points at, or `None` when there is no link.
    ///
    /// `read_link` answers both halves at once — is this a link, and what does it name.
    fn current_target(&self) -> Option<String> {
        fs::read_link(self.current())
            .ok()?
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }

    /// The version history recorded in `install.lock`, oldest first.
    fn lock_history(&self) -> Vec<String> {
        let Ok(lock) = fs::read_to_string(self.lock_file()) else {
            return Vec::new();
        };
        lock.lines()
            .find_map(|line| line.strip_prefix("history="))
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    /// The version directories under `releases/`, newest first: the lock's history (the
    /// authority) reversed, then anything it does not know about — a deleted lock, say — by
    /// modification time.
    fn ordered_dirs(&self) -> Vec<String> {
        let mut ordered: Vec<String> = Vec::new();
        for name in self.lock_history().iter().rev() {
            if !ordered.contains(name) && self.version_dir(name).is_some() {
                ordered.push(name.clone());
            }
        }

        let mut rest = Vec::new();
        if let Ok(entries) = fs::read_dir(self.releases()) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if ordered.contains(&name) || self.version_dir(&name).is_none() {
                    continue;
                }
                let modified = entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                rest.push((modified, name));
            }
        }
        rest.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        ordered.extend(rest.into_iter().map(|(_, name)| name));
        ordered
    }

    /// The directory `--rollback` switches to: the newest one that is not the current one.
    fn rollback_target(&self) -> Option<String> {
        let current = self.current_target();
        self.ordered_dirs()
            .into_iter()
            .find(|name| Some(name.as_str()) != current.as_deref())
    }

    /// The history to record after installing `rel`: every directory that is still there,
    /// oldest first, plus `rel`.
    fn new_history(&self, rel: &str) -> String {
        let mut history: Vec<String> = self
            .ordered_dirs()
            .into_iter()
            .filter(|name| name != rel)
            .collect();
        history.reverse();
        history.push(rel.to_string());
        history.join(" ")
    }

    /// Write `install.lock` through a temporary file, the way install.sh does.
    fn write_lock(&self, rel: &str, number: &str, prev: &str) -> Result<()> {
        let installed_at = OffsetDateTime::now_utc()
            .format(INSTALLED_AT_FMT)
            .unwrap_or_default();
        let body = format!(
            "version={number}\ntarget={TARGET}\ndir=releases/{rel}\ninstalled_at={installed_at}\nprevious={prev}\nhistory={}\n",
            self.new_history(rel)
        );
        let lock = self.lock_file();
        let temporary = lock.with_file_name(format!("install.lock.tmp.{}", std::process::id()));
        if fs::write(&temporary, body).is_err() || fs::rename(&temporary, &lock).is_err() {
            let _ = fs::remove_file(&temporary);
            bail!("写不了 {}（权限不足？）", lock.display());
        }
        Ok(())
    }

    /// Point `current` at `releases/<rel>`.
    ///
    /// The new link is built beside the old one and renamed over it, which replaces the
    /// symlink in one step. If that rename is refused, the old link is removed first and the
    /// link is built in place as a last resort.
    fn switch_current(&self, rel: &str) -> Result<()> {
        let new = self.dir.join(format!(".current.{}", std::process::id()));
        self.remove_link(&new);
        self.create_link(&new, rel)?;

        if fs::rename(&new, self.current()).is_ok() {
            return Ok(());
        }
        self.remove_link(&self.current());
        if fs::rename(&new, self.current()).is_ok() {
            return Ok(());
        }
        let fallback = self.create_link(&self.current(), rel);
        self.remove_link(&new);
        fallback
    }

    /// Put `current` back where it was after a failed switch.
    fn restore_current(&self, rel: &str) -> Result<()> {
        self.remove_link(&self.current());
        self.create_link(&self.current(), rel)
    }

    /// Create the `current`-style link at `dest`, pointing at `releases/<rel>`.
    fn create_link(&self, dest: &Path, rel: &str) -> Result<()> {
        // A relative target, like `install.sh`: the whole install directory can then be moved.
        std::os::unix::fs::symlink(Path::new("releases").join(rel), dest)
            .map_err(|e| eyre!("建不出 {} -> releases/{rel}（{e}）", dest.display()))
    }

    /// Delete the link itself, never what it points at.
    ///
    /// The path is a symlink, so `remove_file` is what removes it. This only runs when there
    /// really is a link, so a stray directory is left alone — and it never recurses.
    fn remove_link(&self, path: &Path) {
        if fs::read_link(path).is_err() {
            return;
        }
        let _ = fs::remove_file(path);
    }

    /// Keep the newest `KEEP` version directories; the one just installed and the one
    /// `current` points at are never touched.
    fn prune_versions(&self, keep_rel: &str) {
        let current = self.current_target();
        let running = std::env::current_exe()
            .ok()
            .and_then(|exe| fs::canonicalize(exe).ok());

        for (index, name) in self.ordered_dirs().iter().enumerate() {
            if index < KEEP || name == keep_rel || Some(name.as_str()) == current.as_deref() {
                continue;
            }
            let path = self.releases().join(name);
            // Never delete the directory the running executable lives in: `update` is usually
            // started through `current`, so this drops what would only ever be noise.
            let canonical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if running
                .as_ref()
                .is_some_and(|exe| exe.starts_with(&canonical))
            {
                continue;
            }
            match fs::remove_dir_all(&path) {
                Ok(()) => println!("  清理       旧版本 {}", path.display()),
                Err(e) => println!("  警告       旧版本 {} 删不掉，先留着：{e}", path.display()),
            }
        }
    }

    /// Switch `current` to `releases/<rel>`, write the lock, prune, and report the result.
    fn finish_install(&self, rel: &str, number: &str) -> Result<()> {
        let prev = self.current_target();

        if self.switch_current(rel).is_err() {
            if let Some(prev) = &prev {
                if self.restore_current(prev).is_ok() {
                    println!("  回退       current 已还原成 releases/{prev}");
                } else {
                    println!(
                        "  警告       旧链接也没能还原，请手动把 {} 指向 releases/{prev}",
                        self.current().display()
                    );
                }
            }
            bail!("切换 current 到 releases/{rel} 失败 —— 安装中止，current 未被改动");
        }

        self.write_lock(rel, number, prev.as_deref().unwrap_or(""))?;
        self.prune_versions(rel);

        println!(
            "  已安装     {}",
            self.releases().join(rel).join(BIN).display()
        );
        if self
            .releases()
            .join(rel)
            .join(format!("{BIN}-gui"))
            .is_file()
        {
            println!(
                "  桌面界面   {}",
                self.current().join(format!("{BIN}-gui")).display()
            );
        }
        println!("  版本       {number}");
        println!(
            "  current    {} -> releases/{rel}",
            self.current().display()
        );
        println!("  可执行     {}", self.current().join(BIN).display());
        match prev.as_deref() {
            Some(prev) if prev == rel => {
                println!("  上一个版本 （没有变化，current 本来就指向它）");
            }
            Some(prev) => println!("  上一个版本 {prev}"),
            None => println!("  上一个版本 （无）"),
        }
        println!(
            "  回滚命令   boxpigma update --dir {} --rollback",
            self.dir.display()
        );
        Ok(())
    }
}

/// The scratch directories one install writes through, removed on every exit path — including
/// a `bail!` halfway through — so a failed run leaves nothing behind but its messages.
struct Scratch {
    /// `<temp>/boxpigma-update-<pid>`: the downloaded archive.
    tmp: PathBuf,
    /// `<releases>/.staging.<pid>`: unpacked here, then renamed into the version directory.
    staging: PathBuf,
    /// `<releases>/.old.<pid>`: the version directory of the same name moved aside.
    old: PathBuf,
}

impl Scratch {
    /// `staging` and `old` sit under `releases/` on purpose: renaming into place is only
    /// atomic within one filesystem. The leading dot is what keeps `ordered_dirs` — and so the
    /// history and the pruning — from seeing either of them.
    fn new(releases: &Path) -> Self {
        let pid = std::process::id();
        Self {
            tmp: std::env::temp_dir().join(format!("boxpigma-update-{pid}")),
            staging: releases.join(format!(".staging.{pid}")),
            old: releases.join(format!(".old.{pid}")),
        }
    }

    /// Where the downloaded archive lives until it is unpacked.
    fn archive(&self, asset: &str) -> PathBuf {
        self.tmp.join(asset)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
        let _ = fs::remove_dir_all(&self.staging);
        let _ = fs::remove_dir_all(&self.old);
    }
}

/// Unpack `archive` into `dest`.
///
/// Entries are trusted exactly as far as "a path inside `dest`": one that climbs out — through
/// `..` or an absolute path — fails the whole extraction instead of being skipped, because a
/// release archive that tries that is not one to install.
fn extract(archive: &Path, dest: &Path) -> Result<()> {
    let asset = archive
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    let file = fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    for entry in tar.entries().map_err(|_| not_a_tarball(&asset))? {
        let mut entry = entry.map_err(|_| not_a_tarball(&asset))?;
        let unpacked = entry.unpack_in(dest).map_err(|_| not_a_tarball(&asset))?;
        if !unpacked {
            bail!("解压失败：{asset} 里有跑到 {} 外面的条目", dest.display());
        }
    }

    Ok(())
}

fn not_a_tarball(asset: &str) -> color_eyre::Report {
    eyre!("解压失败：{asset} 不是有效的 tar.gz")
}

/* -------------------------------------------------------------------------- */
/*                                   Testing                                  */
/* -------------------------------------------------------------------------- */

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        sync::{Arc, Mutex},
    };

    use super::*;

    /// A scratch install directory this test owns; `label` keeps parallel tests apart.
    fn fixture(label: &str) -> Layout {
        let dir = std::env::temp_dir().join(format!(
            "boxpigma-update-test-{}-{label}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let layout = Layout { dir };
        fs::create_dir_all(layout.releases()).expect("create scratch layout");
        layout
    }

    /// Drop a scratch tree: the links go first (never following them into a version
    /// directory), then the tree itself.
    fn cleanup(layout: &Layout) {
        layout.remove_link(&layout.current());
        let _ = fs::remove_dir_all(&layout.dir);
    }

    fn version_dir(layout: &Layout, name: &str) -> PathBuf {
        let path = layout.releases().join(name);
        fs::create_dir_all(&path).expect("create version directory");
        path
    }

    fn write_archive(path: &Path, name: &str, contents: &[u8]) {
        let file = fs::File::create(path).expect("create archive");
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::default(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        // `append_data` would go through `Header::set_path`, which refuses `..` outright,
        // and this helper has to be able to build the archive `extract` is meant to
        // reject — so the name goes into the header directly.
        let raw = name.as_bytes();
        header.as_old_mut().name[..raw.len()].copy_from_slice(raw);
        header.set_cksum();
        tar.append(&header, contents).expect("append entry");
        tar.into_inner()
            .expect("finish the tar stream")
            .finish()
            .expect("finish the compressed stream");
    }

    #[test]
    fn sha256_hex_matches_the_published_format() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn manifest_selects_the_current_platform_and_keeps_its_metadata() {
        let key = platform_key();
        let json = format!(
            r#"{{
                "version": "1.8.0-beta.1",
                "notes": "beta build",
                "pub_date": "2026-10-25T00:00:00Z",
                "platforms": {{
                    "{key}": {{
                        "url": "https://updates.example/boxpigma-{TARGET}.tar.gz",
                        "install_urls": ["https://updates.example/boxpigma-{TARGET}.tar.gz"],
                        "sha256": "abc"
                    }}
                }}
            }}"#
        );
        let manifest: Manifest = serde_json::from_str(&json).expect("parse manifest");
        let release = release_from_manifest(manifest, &key).expect("select platform");
        assert_eq!(release.version, "1.8.0-beta.1");
        assert_eq!(release.notes.as_deref(), Some("beta build"));
        assert_eq!(release.sha256.as_deref(), Some("abc"));
        assert_eq!(
            checksums_url(&release.asset_url),
            "https://updates.example/SHA256SUMS"
        );
        assert_eq!(
            asset_from_url(&release.asset_url).expect("safe asset"),
            asset_name()
        );
    }

    #[test]
    fn mirror_rewrites_manifest_asset_urls() {
        let mirror = "https://proxy.example/https://github.com";
        assert_eq!(
            mirror_url(
                "https://github.com/GBLMX/pigma/releases/download/v1.7.0/boxpigma.tar.gz",
                Some(mirror),
            ),
            "https://proxy.example/https://github.com/GBLMX/boxpigma/releases/download/v1.7.0/boxpigma.tar.gz"
        );
        assert_eq!(
            mirror_url("https://updates.example/boxpigma.tar.gz", Some(mirror)),
            "https://updates.example/boxpigma.tar.gz"
        );
        assert_eq!(
            mirror_url("https://github.com/a", None),
            "https://github.com/a"
        );
    }

    #[test]
    fn normalized_versions_and_assets_cannot_escape_the_install_root() {
        assert_eq!(normalized_version("v1.8.0").expect("version"), "1.8.0");
        assert!(normalized_version("../../evil").is_err());
        assert!(normalized_version("1.8.0/../../evil").is_err());

        assert_eq!(
            asset_from_url("https://updates.example/boxpigma.tar.gz").expect("asset"),
            "boxpigma.tar.gz"
        );
        assert!(asset_from_url("https://updates.example/").is_err());
        assert!(asset_from_url("not a URL").is_err());

        let key = platform_key();
        for (version, url) in [
            ("../../evil", "https://updates.example/boxpigma.tar.gz"),
            ("1.8.0", "https://updates.example/"),
        ] {
            let manifest: Manifest = serde_json::from_str(&format!(
                r#"{{"version":"{version}","platforms":{{"{key}":{{"url":"{url}"}}}}}}"#
            ))
            .expect("parse manifest");
            assert!(
                release_from_manifest(manifest, &key).is_err(),
                "unsafe manifest must be rejected"
            );
        }
    }

    #[test]
    fn semver_order_handles_prereleases() {
        assert!(is_newer("1.8.0", "1.7.0"));
        assert!(is_newer("1.8.0-beta.1", "1.7.0"));
        assert!(is_newer("1.8.0", "1.8.0-beta.1"));
        assert!(!is_newer("1.8.0-beta.1", "1.8.0"));
        assert!(!is_newer("not-a-version", "1.7.0"));
    }

    #[test]
    fn parse_checksums_finds_this_asset_and_nothing_else() {
        let asset = asset_name();
        let body = format!(
            "0000000000000000000000000000000000000000000000000000000000000000  boxpigma-other-target.tar.gz\n\
             ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789  {asset}\n"
        );
        assert_eq!(
            parse_checksums(&body, &asset).as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789")
        );
        assert_eq!(parse_checksums(&body, "boxpigma-nope.tar.gz"), None);
    }

    #[test]
    fn ordered_dirs_follow_the_lock_history_and_skip_scratch_entries() {
        let layout = fixture("ordered");
        for name in ["1.4.0-stale", "1.5.0-mid", "1.6.0-new"] {
            version_dir(&layout, name);
        }
        // History is the authority: it wins over any modification-time ordering.
        fs::write(
            layout.lock_file(),
            "version=1.6.0\nhistory=1.4.0-stale 1.5.0-mid 1.6.0-new\n",
        )
        .unwrap();
        // Neither a scratch directory nor a link is a version.
        fs::create_dir_all(layout.releases().join(".staging.1")).unwrap();
        layout
            .create_link(&layout.releases().join("linked"), "1.4.0-stale")
            .unwrap();
        // A directory the lock has never heard of comes after the ones it knows.
        version_dir(&layout, "unlisted");

        let ordered = layout.ordered_dirs();
        assert_eq!(
            ordered,
            vec![
                "1.6.0-new".to_string(),
                "1.5.0-mid".to_string(),
                "1.4.0-stale".to_string(),
                "unlisted".to_string(),
            ]
        );
        cleanup(&layout);
    }

    #[test]
    fn switch_current_moves_the_link_and_leaves_the_old_version_alone() {
        let layout = fixture("switch");
        let old = version_dir(&layout, "1.5.0-old");
        fs::write(old.join(BIN), b"old binary").unwrap();
        version_dir(&layout, "1.6.0-new");
        layout.create_link(&layout.current(), "1.5.0-old").unwrap();
        assert_eq!(layout.current_target().as_deref(), Some("1.5.0-old"));

        layout.switch_current("1.6.0-new").expect("switch current");

        assert_eq!(layout.current_target().as_deref(), Some("1.6.0-new"));
        // Removing the link must not take the directory it named with it.
        assert_eq!(fs::read(old.join(BIN)).unwrap(), b"old binary");
        cleanup(&layout);
    }

    #[test]
    fn write_lock_round_trips_through_the_history() {
        let layout = fixture("lock");
        version_dir(&layout, "1.5.0-old");
        version_dir(&layout, "1.6.0-new");
        fs::write(
            layout.lock_file(),
            "version=1.5.0\nhistory=1.4.0-gone 1.5.0-old\n",
        )
        .unwrap();

        layout
            .write_lock("1.6.0-new", "1.6.0", "1.5.0-old")
            .expect("write the lock");

        // Oldest first, and the version just installed is last…
        assert_eq!(layout.lock_history(), ["1.5.0-old", "1.6.0-new"]);
        let lock = fs::read_to_string(layout.lock_file()).unwrap();
        // …while the first install of the directory records no predecessor at all, which
        // install.sh writes as an empty `previous=`.
        assert!(lock.contains("\nprevious=1.5.0-old\n"), "{lock}");
        assert!(lock.starts_with("version=1.6.0\n"), "{lock}");
        assert!(lock.contains("\ndir=releases/1.6.0-new\n"), "{lock}");
        assert!(lock.contains(&format!("\ntarget={TARGET}\n")), "{lock}");

        layout.write_lock("1.5.0-old", "1.5.0", "").unwrap();
        let lock = fs::read_to_string(layout.lock_file()).unwrap();
        assert!(lock.contains("\nprevious=\n"), "{lock}");
        cleanup(&layout);
    }

    #[test]
    fn install_dir_from_exe_recognizes_every_layout() {
        let root =
            std::env::temp_dir().join(format!("boxpigma-update-test-{}-exe", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let releases = root.join("releases");
        let rel = format!("1.5.0-{TARGET}");
        fs::create_dir_all(releases.join(&rel)).unwrap();
        fs::create_dir_all(root.join("current")).unwrap();

        // Started through `current`, which is how an installed boxpigma is run…
        assert_eq!(
            install_dir_from_exe(&root.join("current").join(BIN)),
            Some(root.clone())
        );
        // …or straight out of `releases/`, or from the pre-`releases` layout.
        assert_eq!(
            install_dir_from_exe(&releases.join(&rel).join(BIN)),
            Some(root.clone())
        );
        assert_eq!(install_dir_from_exe(&root.join(BIN)), Some(root.clone()));
        // Nothing that looks like an install: no directory to update.
        assert_eq!(
            install_dir_from_exe(&root.join("elsewhere").join(BIN)),
            None
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn version_number_from_dir_strips_the_target() {
        assert_eq!(version_number_from_dir(&format!("1.5.0-{TARGET}")), "1.5.0");
        // A directory that does not follow the convention keeps its name.
        assert_eq!(version_number_from_dir("1.5.0"), "1.5.0");
    }

    #[test]
    fn rollback_target_skips_the_current_version() {
        let layout = fixture("rollback");
        for name in ["1.4.0-old", "1.5.0-mid", "1.6.0-new"] {
            version_dir(&layout, name);
        }
        fs::write(
            layout.lock_file(),
            "history=1.4.0-old 1.5.0-mid 1.6.0-new\n",
        )
        .unwrap();
        layout.create_link(&layout.current(), "1.6.0-new").unwrap();

        assert_eq!(layout.rollback_target().as_deref(), Some("1.5.0-mid"));
        cleanup(&layout);
    }

    #[test]
    fn extract_unpacks_the_binary_and_refuses_to_escape() {
        let layout = fixture("extract");
        let dest = layout.dir.join("out");
        fs::create_dir_all(&dest).unwrap();

        let archive = layout.dir.join(asset_name());
        write_archive(&archive, BIN, b"boxpigma binary");
        extract(&archive, &dest).expect("extract a release archive");
        assert_eq!(fs::read(dest.join(BIN)).unwrap(), b"boxpigma binary");
        use std::os::unix::fs::PermissionsExt;
        // The header's mode survives, minus the umask: the archive's own 0o755 can only
        // lose bits the umask clears, and the caller chmods 0o755 on top of it.
        let mode = fs::metadata(dest.join(BIN)).unwrap().permissions().mode();
        assert!(
            mode & 0o100 != 0,
            "unpacked binary is not executable: {mode:o}"
        );

        // An entry that climbs out fails the extraction instead of being skipped.
        let escaping = layout.dir.join(format!("escaping.{}", asset_name()));
        write_archive(&escaping, "../escaped", b"nope");
        assert!(extract(&escaping, &dest).is_err());
        assert!(!layout.dir.join("escaped").exists());
        cleanup(&layout);
    }

    /// A loopback release host with the shape `latest_tag` and the asset probe expect:
    /// `…/releases/latest` answers 302 to `…/releases/tag/<tag>`, that tag path answers 200,
    /// and everything else — the asset probe included — answers 404.
    ///
    /// Every requested path is recorded, which is what lets a test pin *which* host was asked:
    /// a URL built from the constant instead of the argument never reaches this server. The
    /// listener is on the loopback interface only; the thread is detached and dies with the
    /// test process.
    fn release_host(tag: &str) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let host = format!(
            "http://{}",
            listener.local_addr().expect("loopback address")
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        let base = host.clone();
        let tag = tag.to_string();

        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
                    continue;
                }
                // Drain the headers up to the blank line. `HEAD` never has a body, so there is
                // nothing else to wait for.
                loop {
                    let mut header = String::new();
                    match reader.read_line(&mut header) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if header == "\r\n" => break,
                        Ok(_) => {}
                    }
                }

                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                recorder
                    .lock()
                    .expect("record the request")
                    .push(path.clone());

                let response = if path == "/GBLMX/pigma/releases/latest" {
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: {base}/GBLMX/pigma/releases/tag/{tag}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else if path.starts_with("/GBLMX/pigma/releases/tag/") {
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                };
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        (host, seen)
    }

    fn manifest_host(body: String) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let host = format!(
            "http://{}",
            listener.local_addr().expect("loopback address")
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);

        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().expect("clone the stream"));
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
                    continue;
                }
                loop {
                    let mut header = String::new();
                    match reader.read_line(&mut header) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if header == "\r\n" => break,
                        Ok(_) => {}
                    }
                }
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                recorder.lock().expect("record the request").push(path);

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });

        (host, seen)
    }

    #[tokio::test]
    async fn fetch_release_reads_an_explicit_manifest_before_github() {
        let key = platform_key();
        let body = format!(
            r#"{{"version":"1.8.0","platforms":{{"{key}":{{"url":"https://updates.example/boxpigma-{TARGET}.tar.gz","sha256":"abc"}}}}}}"#
        );
        let (host, seen) = manifest_host(body);
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = http_client().expect("http client");
        let mut opts = opts_for(host.clone(), false, false);
        opts.manifest = Some(format!("{host}/boxpigma-updates.json"));

        let release = fetch_release(&client, &opts)
            .await
            .expect("read explicit manifest");
        assert_eq!(release.version, "1.8.0");
        assert_eq!(release.sha256.as_deref(), Some("abc"));
        assert_eq!(
            release.asset_url,
            format!("https://updates.example/boxpigma-{TARGET}.tar.gz")
        );
        assert!(
            seen.lock()
                .expect("recorded requests")
                .iter()
                .any(|path| path == "/boxpigma-updates.json")
        );
    }

    /// `--check` has to ask the mirror: `--mirror` reaches both requests, the release lookup
    /// and the asset probe. Both used to name `HOST` directly, so a mirror user — precisely
    /// someone who cannot reach `github.com` — got a `--check` that could only fail.
    #[tokio::test]
    async fn check_asks_the_mirror_for_both_the_release_and_the_asset() {
        let (host, seen) = release_host("v9.9.9");
        // Same one-liner `main.rs` runs at startup; `http_client` builds a `rustls-no-provider`
        // client and would panic without it.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = http_client().expect("http client");

        assert_eq!(
            latest_tag(&client, &host).await.expect("read the tag"),
            "9.9.9"
        );

        // A version directory with no binary in it: `check` cannot read an installed version,
        // so it goes on to the asset probe instead of returning early.
        let layout = fixture("check-mirror");
        let rel = format!("1.5.0-{TARGET}");
        version_dir(&layout, &rel);
        layout
            .create_link(&layout.current(), &rel)
            .expect("point current at the installed version");

        check(&client, &layout, &opts_for(host.clone(), false, false))
            .await
            .expect("check");

        let paths = seen.lock().expect("read the recorded paths").clone();
        let asset_path = format!("/GBLMX/pigma/releases/latest/download/{}", asset_name());
        assert!(
            paths
                .iter()
                .any(|path| path == "/GBLMX/pigma/releases/latest"),
            "the release lookup never reached {host}: {paths:?}"
        );
        assert!(
            paths.iter().any(|path| path == &asset_path),
            "the asset probe never reached {host}: {paths:?}"
        );
        cleanup(&layout);
    }

    /// The options a test's `install` call needs; the two switches these tests are about are
    /// left to the caller.
    fn opts_for(host: String, dry_run: bool, insecure: bool) -> Options {
        Options {
            check: false,
            version: "latest".into(),
            dir: None,
            mirror: Some(host.clone()),
            checksums: None,
            force: false,
            insecure,
            dry_run,
            rollback: false,
            channel: UpdateChannel::Stable,
            manifest: Some(format!("{host}/boxpigma-updates.json")),
        }
    }

    /// No `SHA256SUMS`, no install. The published list is the only thing that makes the download
    /// trustworthy, and a mirror user — the one most likely to be missing it — must not get an
    /// unverified binary by accident. The refusal has to name both ways out, and it has to
    /// happen before anything is written.
    #[tokio::test]
    async fn install_refuses_when_the_checksums_cannot_be_fetched() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = http_client().expect("http client");
        let layout = fixture("no-checksums");
        let (host, _seen) = release_host("v1.6.0");

        let error = install(&client, &layout, &opts_for(host, false, false))
            .await
            .expect_err("must refuse without a checksum list");
        let message = error.to_string();
        assert!(message.contains("--checksums"), "{message}");
        assert!(message.contains("--insecure"), "{message}");
        assert_eq!(
            fs::read_dir(layout.releases())
                .expect("read releases")
                .count(),
            0,
            "a refused run must not write a version directory"
        );
        cleanup(&layout);
    }

    /// `--dry-run` exists to show the plan, so a missing list is reported rather than fatal:
    /// that a real run would refuse is part of the plan.
    #[tokio::test]
    async fn dry_run_reports_a_missing_checksum_list_without_failing() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = http_client().expect("http client");
        let layout = fixture("no-checksums-dry");
        let (host, _seen) = release_host("v1.6.0");

        install(&client, &layout, &opts_for(host, true, false))
            .await
            .expect("dry-run must not fail on a missing checksum list");
        cleanup(&layout);
    }
}
