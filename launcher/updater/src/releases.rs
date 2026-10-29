//! GitHub Releases client with a cache: `GET /repos/<owner>/<name>/releases/latest` with `If-None-Match`, so repeated starts cost nothing
//! against the unauthenticated limit of 60 requests per hour.
//!
//! The cache file (`cache/latest.json` in the launcher's per-user folder) holds the last answer, its ETag, the manifest that belongs to it
//! and the time of the last successful contact. Offline, rate-limited or broken answers fall back to it: the notes still show, the
//! state says "could not check", and nothing here ever blocks Play.

use crate::config::ReleaseConfig;
use crate::error::{Kind, Result, UpdateError};
use crate::http::{Http, API_TIMEOUT};
use crate::manifest::ReleaseManifest;
use crate::{now_unix, version};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_RELEASE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_LIST_BYTES: usize = 8 * 1024 * 1024;
pub const LIST_LIMIT: usize = 20;
const CACHE_SCHEMA: u32 = 1;
const CACHE_FILE: &str = "latest.json";
/// Without a reset time from the server, stay away from the API for this long after a rate-limit answer.
const DEFAULT_BLOCK_SECS: i64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub body: String,
    pub published_at: Option<String>,
    pub html_url: String,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn version(&self) -> Option<String> {
        version::normalise(&self.tag)
    }
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

/// Parses one release object of the GitHub API. Assets on hosts that are not allowed are dropped; `(release, is_draft_or_prerelease)`.
fn release_from_raw(raw: RawRelease, cfg: &ReleaseConfig) -> Result<(Release, bool)> {
    if version::parse(&raw.tag_name).is_none() {
        return Err(UpdateError::new(Kind::BadResponse, format!("release tag is not a version: {}", raw.tag_name.chars().take(40).collect::<String>())));
    }
    let html_url = match raw.html_url.as_deref() {
        Some(u) if cfg.check_url(u).is_ok() => u.to_string(),
        _ => cfg.web_release_url(&raw.tag_name),
    };
    let assets = raw
        .assets
        .into_iter()
        .filter(|a| {
            let ok = cfg.check_asset_url(&a.browser_download_url).is_ok();
            if !ok {
                log::warn!("release asset {} dropped: its URL is not allowed", a.name.chars().take(60).collect::<String>());
            }
            ok
        })
        .map(|a| Asset { name: a.name, size: a.size, url: a.browser_download_url })
        .collect();
    let rel = Release {
        name: raw.name.filter(|n| !n.is_empty()).unwrap_or_else(|| raw.tag_name.clone()),
        tag: raw.tag_name,
        body: raw.body.unwrap_or_default(),
        published_at: raw.published_at,
        html_url,
        assets,
    };
    Ok((rel, raw.draft || raw.prerelease))
}

pub fn parse_release(bytes: &[u8], cfg: &ReleaseConfig) -> Result<Release> {
    let raw: RawRelease = serde_json::from_slice(bytes).map_err(|e| UpdateError::new(Kind::BadResponse, format!("release JSON: {e}")))?;
    Ok(release_from_raw(raw, cfg)?.0)
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cache {
    pub schema: u32,
    pub etag: Option<String>,
    /// Unix seconds of the last answer from GitHub (200 or 304); 0 = never.
    pub checked_at: i64,
    /// Do not call the API before this time (rate limit).
    pub blocked_until: i64,
    /// The last `releases/latest` body, as received.
    pub release_json: Option<String>,
    /// manifest.json of that release, as received.
    pub manifest_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CheckState {
    /// A 200 from the API: everything is current.
    Fresh,
    /// A 304: the cached answer is still the latest.
    NotModified,
    /// No network. The cached answer (if any) is shown.
    Offline,
    RateLimited { reset_at: Option<i64> },
    /// Anything else (`code` = [`Kind::code`]).
    Failed { code: String, detail: String },
    /// No call was made (checking is off, or this is the cached view at start).
    NotChecked,
}

#[derive(Debug, Clone)]
pub struct Checked {
    pub state: CheckState,
    pub release: Option<Release>,
    pub manifest: Option<ReleaseManifest>,
    /// Unix seconds of the last time GitHub answered; 0 = never.
    pub checked_at: i64,
    /// Why the release's manifest is unusable, when the release itself was read.
    pub manifest_error: Option<String>,
}

#[derive(Clone)]
pub struct Releases {
    http: Http,
    dir: PathBuf,
}

fn cache_path(dir: &Path) -> PathBuf {
    dir.join(CACHE_FILE)
}

pub fn load_cache(dir: &Path) -> Cache {
    match std::fs::read(cache_path(dir)) {
        Ok(b) => match serde_json::from_slice::<Cache>(&b) {
            Ok(c) if c.schema == CACHE_SCHEMA => c,
            Ok(_) => Cache::default(),
            Err(e) => {
                log::warn!("release cache is damaged ({e}); ignoring it");
                Cache::default()
            }
        },
        Err(_) => Cache::default(),
    }
}

pub fn save_cache(dir: &Path, c: &Cache) {
    let mut c2 = Cache { schema: CACHE_SCHEMA, etag: c.etag.clone(), checked_at: c.checked_at, blocked_until: c.blocked_until, release_json: c.release_json.clone(), manifest_json: c.manifest_json.clone() };
    c2.schema = CACHE_SCHEMA;
    match serde_json::to_vec_pretty(&c2) {
        Ok(b) => {
            if let Err(e) = crate::write_atomic(&cache_path(dir), &b) {
                log::warn!("cannot write the release cache: {e}");
            }
        }
        Err(e) => log::warn!("cannot serialise the release cache: {e}"),
    }
}

impl Releases {
    pub fn new(http: Http, cache_dir: impl Into<PathBuf>) -> Releases {
        Releases { http, dir: cache_dir.into() }
    }

    pub fn cfg(&self) -> &ReleaseConfig {
        &self.http.cfg
    }

    pub fn http(&self) -> &Http {
        &self.http
    }

    fn from_cache(&self, c: &Cache, state: CheckState) -> Checked {
        let cfg = self.cfg();
        let release = c.release_json.as_ref().and_then(|j| parse_release(j.as_bytes(), cfg).ok());
        let mut manifest_error = None;
        let manifest = match (&release, &c.manifest_json) {
            (Some(rel), Some(m)) => match ReleaseManifest::parse(m, cfg) {
                Ok(m) if rel.version().as_deref() == Some(m.version.as_str()) => Some(m),
                Ok(_) => {
                    manifest_error = Some("the cached manifest is for another version".to_string());
                    None
                }
                Err(e) => {
                    manifest_error = Some(e.to_string());
                    None
                }
            },
            _ => None,
        };
        Checked { state, release, manifest, checked_at: c.checked_at, manifest_error }
    }

    /// What the cache holds, without touching the network (the first thing the Home page shows).
    pub fn cached(&self) -> Checked {
        self.from_cache(&load_cache(&self.dir), CheckState::NotChecked)
    }

    /// Asks GitHub for the latest release (conditional request) and its manifest. Never fails: every problem is a [`CheckState`] and the
    /// cached answer (if there is one) comes back with it. When the API cannot be used (no connection to `api.github.com`, a rate limit, an
    /// HTTP error) the release's manifest.json is fetched through github.com's own `releases/latest/download/` redirect instead: some
    /// networks block the API host and not github.com (GitHub #20).
    pub async fn check_latest(&self) -> Checked {
        let c = self.check_api().await;
        let usable = matches!(&c.state, CheckState::Offline | CheckState::RateLimited { .. }) || matches!(&c.state, CheckState::Failed { code, .. } if code == Kind::Http.code());
        if !usable {
            return c;
        }
        match self.check_web().await {
            Some(w) => w,
            None => c,
        }
    }

    /// The latest release without the API: manifest.json from `<web_base>/<repo>/releases/latest/download/` (a redirect to the newest
    /// published release's asset; drafts and pre-releases are not served there). The manifest's signature is checked as always, and the
    /// release is rebuilt from it: tag `v<version>`, the game package and the launcher by the names the manifest gives, no notes text.
    async fn check_web(&self) -> Option<Checked> {
        let cfg = self.cfg();
        let url = format!("{}/{}/releases/latest/download/{}", cfg.web_base.trim_end_matches('/'), cfg.repo, cfg.manifest_asset);
        let reply = match self.http.get(&url, None, "application/octet-stream", crate::manifest::MAX_MANIFEST_BYTES, API_TIMEOUT).await {
            Ok(r) if r.status == 200 => r,
            Ok(r) => {
                log::info!("update check through github.com: HTTP {}", r.status);
                return None;
            }
            Err(e) => {
                log::info!("update check through github.com: {e}");
                return None;
            }
        };
        let text = String::from_utf8(reply.body).ok()?;
        let m = match ReleaseManifest::parse(&text, cfg) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("update check through github.com: manifest refused: {e}");
                return None;
            }
        };
        let rel = release_from_manifest(cfg, &m);
        let now = now_unix();
        let mut cache = load_cache(&self.dir);
        cache.checked_at = now;
        cache.blocked_until = 0;
        cache.etag = None;
        cache.release_json = Some(release_json_of(&rel));
        cache.manifest_json = Some(text);
        save_cache(&self.dir, &cache);
        log::info!("update check through github.com (the API was not usable): latest is {} (manifest ok)", rel.tag);
        Some(Checked { state: CheckState::Fresh, release: Some(rel), manifest: Some(m), checked_at: now, manifest_error: None })
    }

    async fn check_api(&self) -> Checked {
        let mut cache = load_cache(&self.dir);
        let now = now_unix();
        if cache.blocked_until > now {
            log::info!("update check skipped: rate limited until {}", cache.blocked_until);
            return self.from_cache(&cache, CheckState::RateLimited { reset_at: Some(cache.blocked_until) });
        }
        let url = self.cfg().api_url("releases/latest");
        // only ask "has it changed" if we can actually use the cached copy
        let cached_ok = cache.release_json.as_ref().map(|j| parse_release(j.as_bytes(), self.cfg()).is_ok()).unwrap_or(false) && cache.manifest_json.is_some();
        let etag = if cached_ok { cache.etag.clone() } else { None };
        let reply = match self.http.get(&url, etag.as_deref(), "application/vnd.github+json", MAX_RELEASE_BYTES, API_TIMEOUT).await {
            Ok(r) => r,
            Err(e) => {
                log::info!("update check: {e}");
                let state = if e.kind == Kind::Offline { CheckState::Offline } else { CheckState::Failed { code: e.kind.code().into(), detail: e.detail } };
                return self.from_cache(&cache, state);
            }
        };
        match reply.status {
            304 => {
                cache.checked_at = now;
                save_cache(&self.dir, &cache);
                log::info!("update check: 304 not modified");
                self.from_cache(&cache, CheckState::NotModified)
            }
            200 => self.take_fresh(cache, reply.body, reply.etag, now).await,
            429 | 403 => {
                let text = String::from_utf8_lossy(&reply.body).to_ascii_lowercase();
                let limited = reply.status == 429 || reply.rate_remaining == Some(0) || text.contains("rate limit");
                if limited {
                    let until = reply.rate_reset.filter(|t| *t > now).unwrap_or(now + DEFAULT_BLOCK_SECS);
                    cache.blocked_until = until;
                    save_cache(&self.dir, &cache);
                    log::warn!("update check: rate limited until {until}");
                    self.from_cache(&cache, CheckState::RateLimited { reset_at: Some(until) })
                } else {
                    self.from_cache(&cache, CheckState::Failed { code: Kind::Http.code().into(), detail: format!("HTTP {}", reply.status) })
                }
            }
            s => {
                log::warn!("update check: HTTP {s}");
                self.from_cache(&cache, CheckState::Failed { code: Kind::Http.code().into(), detail: format!("HTTP {s}") })
            }
        }
    }

    async fn take_fresh(&self, mut cache: Cache, body: Vec<u8>, etag: Option<String>, now: i64) -> Checked {
        let cfg = self.cfg();
        let release = match parse_release(&body, cfg) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("update check: {e}");
                return self.from_cache(&cache, CheckState::Failed { code: e.kind.code().into(), detail: e.detail });
            }
        };
        let text = String::from_utf8_lossy(&body).into_owned();
        // the manifest belonging to this release: reuse the cached one when it is for this very tag
        let reuse = match (&cache.release_json, &cache.manifest_json) {
            (Some(old), Some(m)) if parse_release(old.as_bytes(), cfg).map(|r| r.tag == release.tag).unwrap_or(false) => Some(m.clone()),
            _ => None,
        };
        let manifest_text = match reuse {
            Some(m) => Ok(m),
            None => self.fetch_manifest_text(&release).await,
        };
        cache.checked_at = now;
        cache.blocked_until = 0;
        cache.release_json = Some(text);
        match manifest_text.and_then(|t| ReleaseManifest::parse(&t, cfg).and_then(|m| check_manifest_matches(&release, &m).map(|_| (t, m)))) {
            Ok((t, m)) => {
                cache.etag = etag;
                cache.manifest_json = Some(t);
                save_cache(&self.dir, &cache);
                log::info!("update check: latest is {} (manifest ok)", release.tag);
                Checked { state: CheckState::Fresh, release: Some(release), manifest: Some(m), checked_at: now, manifest_error: None }
            }
            Err(e) => {
                // the notes are still worth showing; no ETag, so the next check asks again instead of getting a 304
                cache.etag = None;
                cache.manifest_json = None;
                save_cache(&self.dir, &cache);
                log::warn!("update check: release {} has no usable manifest: {e}", release.tag);
                Checked { state: CheckState::Failed { code: e.kind.code().into(), detail: e.detail.clone() }, release: Some(release), manifest: None, checked_at: now, manifest_error: Some(e.to_string()) }
            }
        }
    }

    async fn fetch_manifest_text(&self, rel: &Release) -> Result<String> {
        let name = &self.cfg().manifest_asset;
        let asset = rel.asset(name).ok_or_else(|| UpdateError::new(Kind::Missing, format!("release {} has no {name}", rel.tag)))?;
        let reply = self.http.get(&asset.url, None, "application/octet-stream", crate::manifest::MAX_MANIFEST_BYTES, API_TIMEOUT).await?;
        if reply.status != 200 {
            return Err(UpdateError::new(Kind::Http, format!("{name} answered HTTP {}", reply.status)));
        }
        String::from_utf8(reply.body).map_err(|_| UpdateError::new(Kind::Manifest, "manifest is not UTF-8 text"))
    }

    /// The manifest of a given release (for "install a specific version").
    pub async fn manifest_for(&self, rel: &Release) -> Result<ReleaseManifest> {
        let text = self.fetch_manifest_text(rel).await?;
        let m = ReleaseManifest::parse(&text, self.cfg())?;
        check_manifest_matches(rel, &m)?;
        Ok(m)
    }

    /// Up to `limit` (at most 20) published releases, newest first. Drafts and pre-releases are left out (there are no channels).
    pub async fn list(&self, limit: usize) -> Result<Vec<Release>> {
        let limit = limit.clamp(1, LIST_LIMIT);
        let url = self.cfg().api_url(&format!("releases?per_page={LIST_LIMIT}"));
        let reply = self.http.get(&url, None, "application/vnd.github+json", MAX_LIST_BYTES, API_TIMEOUT).await?;
        match reply.status {
            200 => {}
            429 | 403 => {
                let mut e = UpdateError::new(Kind::RateLimited, format!("HTTP {}", reply.status));
                e.reset_at = reply.rate_reset;
                return Err(e);
            }
            s => return Err(UpdateError::new(Kind::Http, format!("HTTP {s}"))),
        }
        let raws: Vec<serde_json::Value> = serde_json::from_slice(&reply.body).map_err(|e| UpdateError::new(Kind::BadResponse, format!("release list: {e}")))?;
        let mut out: Vec<Release> = Vec::new();
        for v in raws.into_iter().take(LIST_LIMIT * 2) {
            let Ok(raw) = serde_json::from_value::<RawRelease>(v) else { continue };
            let Ok((rel, hidden)) = release_from_raw(raw, self.cfg()) else { continue };
            if !hidden {
                out.push(rel);
            }
        }
        out.sort_by(|a, b| version::parse(&b.tag).cmp(&version::parse(&a.tag)));
        out.truncate(limit);
        Ok(out)
    }
}

/// The release a manifest belongs to, for a check that did not use the API: the asset URLs are the release downloads of this repository.
fn release_from_manifest(cfg: &ReleaseConfig, m: &ReleaseManifest) -> Release {
    let tag = format!("v{}", m.version);
    let dl = |name: &str| format!("{}/{}/releases/download/{}/{}", cfg.web_base.trim_end_matches('/'), cfg.repo, tag, name);
    Release {
        name: tag.clone(),
        body: String::new(),
        published_at: None,
        html_url: cfg.web_release_url(&tag),
        assets: vec![
            Asset { name: cfg.manifest_asset.clone(), size: 0, url: dl(&cfg.manifest_asset) },
            Asset { name: m.game.asset.clone(), size: m.game.size, url: dl(&m.game.asset) },
            Asset { name: m.launcher.asset.clone(), size: m.launcher.size, url: dl(&m.launcher.asset) },
        ],
        tag,
    }
}

/// The release as the API's JSON, so the cache (which holds the API's shape) can read it back.
fn release_json_of(r: &Release) -> String {
    serde_json::json!({
        "tag_name": r.tag, "name": r.name, "body": r.body, "html_url": r.html_url, "draft": false, "prerelease": false,
        "assets": r.assets.iter().map(|a| serde_json::json!({"name": a.name, "size": a.size, "browser_download_url": a.url})).collect::<Vec<_>>(),
    })
    .to_string()
}

fn check_manifest_matches(rel: &Release, m: &ReleaseManifest) -> Result<()> {
    if rel.version().as_deref() != Some(m.version.as_str()) {
        return Err(UpdateError::new(Kind::Manifest, format!("manifest.json is for {} but the release is {}", m.version, rel.tag)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_damaged_cache_is_an_empty_cache() {
        let d = tempfile::tempdir().unwrap();
        assert!(load_cache(d.path()).release_json.is_none());
        std::fs::write(d.path().join(CACHE_FILE), "{ not json").unwrap();
        assert!(load_cache(d.path()).etag.is_none());
        // a cache written by a different schema is not trusted either
        std::fs::write(d.path().join(CACHE_FILE), r#"{"schema": 99, "etag": "\"x\"", "checked_at": 5}"#).unwrap();
        assert!(load_cache(d.path()).etag.is_none());
    }

    #[test]
    fn the_cache_round_trips_with_etag_and_times() {
        let d = tempfile::tempdir().unwrap();
        let c = Cache { schema: CACHE_SCHEMA, etag: Some("\"abc\"".into()), checked_at: 1234, blocked_until: 99, release_json: Some("{}".into()), manifest_json: Some("m".into()) };
        save_cache(d.path(), &c);
        let back = load_cache(d.path());
        assert_eq!((back.etag.as_deref(), back.checked_at, back.blocked_until), (Some("\"abc\""), 1234, 99));
        assert!(!d.path().join("latest.json.tmp").exists(), "written through a temporary file that is renamed");
    }

    #[test]
    fn release_objects_are_read_strictly_enough() {
        let cfg = ReleaseConfig::embedded("0.1.0");
        let ok = br#"{"tag_name":"v1.2.3","name":"N","body":"B","published_at":"2026-01-01T00:00:00Z","assets":[
            {"name":"a.zip","size":5,"browser_download_url":"https://github.com/holdmysocks/rusty-mcla/releases/download/v1.2.3/a.zip"},
            {"name":"b.zip","size":5,"browser_download_url":"https://evil.example/b.zip"}]}"#;
        let r = parse_release(ok, &cfg).unwrap();
        assert_eq!((r.tag.as_str(), r.version().as_deref(), r.assets.len()), ("v1.2.3", Some("1.2.3"), 1));
        assert_eq!(r.html_url, "https://github.com/holdmysocks/rusty-mcla/releases/tag/v1.2.3", "a missing html_url falls back to the configured repository");
        assert!(parse_release(br#"{"tag_name":"latest"}"#, &cfg).is_err());
        assert!(parse_release(br#"{"name":"x"}"#, &cfg).is_err());
        assert!(parse_release(b"[]", &cfg).is_err());
    }
}
