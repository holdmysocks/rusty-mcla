//! Integration tests: the whole update system against a local mock of the GitHub API.
//! 200 with release + manifest + zip, 304, rate limit, malformed JSON, wrong hash, truncated download, offline, redirects, self-update.

mod common;

use common::*;
use mcla_updater::config::ReleaseConfig;
use mcla_updater::http::Http;
use mcla_updater::package::{self, Options};
use mcla_updater::plan::{plan, Installed};
use mcla_updater::releases::{CheckState, Releases};
use mcla_updater::{selfupdate, Cancel, Kind};
use std::path::PathBuf;
use std::sync::Arc;

struct Rig {
    mock: MockServer,
    key: TestKey,
    root: tempfile::TempDir,
    user: tempfile::TempDir,
    releases: Releases,
    http: Http,
}

fn rig() -> Rig {
    let mock = MockServer::start();
    let key = new_key();
    let cfg = Arc::new(ReleaseConfig::for_mock(&mock.base, Some(key.public_text.clone())));
    let http = Http::new(cfg).unwrap();
    let user = tempfile::tempdir().unwrap();
    let releases = Releases::new(http.clone(), user.path().join("cache"));
    Rig { mock, key, root: tempfile::tempdir().unwrap(), user, releases, http }
}

fn old_game(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("game")).unwrap();
    std::fs::write(root.join("game").join("mcla.exe"), "OLD GAME").unwrap();
}

fn save(user: &std::path::Path) {
    std::fs::create_dir_all(user.join("saves")).unwrap();
    std::fs::write(user.join("saves").join("mcla_save.json"), "{\"career\":1}").unwrap();
}

fn leftovers(root: &std::path::Path) -> Vec<String> {
    ["game.new", ".update", "game.old"].iter().filter(|n| root.join(n).exists()).map(|s| s.to_string()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn latest_200_gives_release_and_manifest_then_304_costs_one_request() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "## Added\n- a thing");
    fx.publish(&r.mock, "\"etag-1\"");

    let c = r.releases.check_latest().await;
    assert_eq!(c.state, CheckState::Fresh);
    assert_eq!(c.release.as_ref().unwrap().tag, "v0.2.0");
    assert_eq!(c.manifest.as_ref().unwrap().game.version, "0.2.0");
    assert!(c.checked_at > 0);
    assert!(r.user.path().join("cache").join("latest.json").is_file());
    let first = r.mock.hits(&fx.paths.latest);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].if_none_match, None);

    // second check: conditional, the server answers 304, the manifest is not fetched again
    let c2 = r.releases.check_latest().await;
    assert_eq!(c2.state, CheckState::NotModified);
    assert_eq!(c2.release.as_ref().unwrap().tag, "v0.2.0");
    assert_eq!(c2.manifest.as_ref().unwrap().version, "0.2.0");
    let hits = r.mock.hits(&fx.paths.latest);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[1].if_none_match.as_deref(), Some("\"etag-1\""));
    assert_eq!(r.mock.hits(&fx.paths.manifest).len(), 1);

    // a newer release changes the ETag: 200 again, new manifest
    let fx2 = fixture(&r.mock.base, "0.3.0", &r.key, 2, "newer");
    fx2.publish(&r.mock, "\"etag-2\"");
    let c3 = r.releases.check_latest().await;
    assert_eq!(c3.state, CheckState::Fresh);
    assert_eq!(c3.manifest.unwrap().game.data_version, 2);
    assert_eq!(r.mock.hits(&fx2.paths.manifest).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn rate_limit_falls_back_to_the_cache_and_stops_asking() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "notes");
    fx.publish(&r.mock, "\"e1\"");
    assert_eq!(r.releases.check_latest().await.state, CheckState::Fresh);

    let reset = mcla_updater::now_unix() + 1800;
    r.mock.set(&fx.paths.latest, Route::Status { code: 403, headers: vec![("x-ratelimit-remaining".into(), "0".into()), ("x-ratelimit-reset".into(), reset.to_string())], body: b"{\"message\":\"API rate limit exceeded\"}".to_vec() });
    let c = r.releases.check_latest().await;
    assert_eq!(c.state, CheckState::RateLimited { reset_at: Some(reset) });
    assert_eq!(c.release.as_ref().unwrap().tag, "v0.2.0", "the cached notes are still there");
    assert!(c.manifest.is_some());

    // blocked: no further request until the reset time
    r.mock.clear_hits();
    let c2 = r.releases.check_latest().await;
    assert_eq!(c2.state, CheckState::RateLimited { reset_at: Some(reset) });
    assert_eq!(r.mock.hits(&fx.paths.latest).len(), 0, "the API is left alone (github.com's own redirect is asked instead)");
}

#[tokio::test(flavor = "multi_thread")]
async fn status_429_with_retry_after_is_a_rate_limit_too() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    r.mock.set(&fx.paths.latest, Route::Status { code: 429, headers: vec![("retry-after".into(), "120".into())], body: vec![] });
    let c = r.releases.check_latest().await;
    match c.state {
        CheckState::RateLimited { reset_at: Some(t) } => assert!(t >= mcla_updater::now_unix() + 100 && t <= mcla_updater::now_unix() + 130),
        s => panic!("{s:?}"),
    }
    assert!(c.release.is_none(), "nothing cached yet");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_403_or_500_is_a_failure_not_a_rate_limit() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    r.mock.set(&fx.paths.latest, Route::Status { code: 403, headers: vec![], body: b"forbidden".to_vec() });
    assert!(matches!(r.releases.check_latest().await.state, CheckState::Failed { .. }));
    r.mock.set(&fx.paths.latest, Route::Status { code: 500, headers: vec![], body: vec![] });
    assert!(matches!(r.releases.check_latest().await.state, CheckState::Failed { .. }));
    r.mock.remove(&fx.paths.latest); // 404: no release yet
    match r.releases.check_latest().await.state {
        CheckState::Failed { code, .. } => assert_eq!(code, "http"),
        s => panic!("{s:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_json_is_refused_and_the_cache_survives() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "good notes");
    fx.publish(&r.mock, "\"e1\"");
    assert_eq!(r.releases.check_latest().await.state, CheckState::Fresh);
    let before = std::fs::read(r.user.path().join("cache").join("latest.json")).unwrap();

    r.mock.set(&fx.paths.latest, Route::Ok { body: b"{ this is not json".to_vec(), etag: Some("\"e2\"".into()) });
    let c = r.releases.check_latest().await;
    match &c.state {
        CheckState::Failed { code, .. } => assert_eq!(code, "bad_response"),
        s => panic!("{s:?}"),
    }
    assert_eq!(c.release.unwrap().body, "good notes");
    assert_eq!(std::fs::read(r.user.path().join("cache").join("latest.json")).unwrap(), before, "a bad answer does not overwrite the cache");

    // not even an object, and a tag that is not a version
    r.mock.set(&fx.paths.latest, Route::Ok { body: b"[1,2,3]".to_vec(), etag: None });
    assert!(matches!(r.releases.check_latest().await.state, CheckState::Failed { .. }));
    r.mock.set(&fx.paths.latest, Route::Ok { body: br#"{"tag_name":"nightly","assets":[]}"#.to_vec(), etag: None });
    assert!(matches!(r.releases.check_latest().await.state, CheckState::Failed { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_answer_is_refused() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    r.mock.set(&fx.paths.latest, Route::Ok { body: vec![b' '; 3 * 1024 * 1024], etag: None });
    match r.releases.check_latest().await.state {
        CheckState::Failed { code, .. } => assert_eq!(code, "too_large"),
        s => panic!("{s:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_release_with_a_bad_manifest_still_shows_its_notes_but_offers_no_update() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "notes of a broken release");
    fx.publish(&r.mock, "\"e1\"");
    r.mock.set(&fx.paths.manifest, Route::Ok { body: br#"{"schema": 2}"#.to_vec(), etag: None });
    let c = r.releases.check_latest().await;
    match &c.state {
        CheckState::Failed { code, .. } => assert_eq!(code, "manifest"),
        s => panic!("{s:?}"),
    }
    assert_eq!(c.release.unwrap().body, "notes of a broken release");
    assert!(c.manifest.is_none());
    // no ETag was stored, so the next check asks again (and sees the fixed manifest)
    r.mock.set(&fx.paths.manifest, Route::Ok { body: fx.manifest_json.clone().into_bytes(), etag: None });
    let c2 = r.releases.check_latest().await;
    assert_eq!(c2.state, CheckState::Fresh);
    assert!(c2.manifest.is_some());

    // a manifest for another version than the release is refused too
    let other = fixture(&r.mock.base, "0.9.0", &r.key, 1, "x");
    r.mock.set(&fx.paths.manifest, Route::Ok { body: other.manifest_json.into_bytes(), etag: None });
    let fx3 = fixture(&r.mock.base, "0.4.0", &r.key, 1, "y");
    fx3.publish(&r.mock, "\"e9\"");
    r.mock.set(&fx3.paths.manifest, Route::Ok { body: fx.manifest_json.clone().into_bytes(), etag: None });
    let c3 = r.releases.check_latest().await;
    assert!(c3.manifest.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_shows_the_cached_notes_and_says_so() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "cached notes");
    fx.publish(&r.mock, "\"e1\"");
    let first = r.releases.check_latest().await;
    assert_eq!(first.state, CheckState::Fresh);

    // the server goes away: same configuration, nothing listening on that port any more
    let base = r.mock.base.clone();
    drop(r.mock);
    let cfg = Arc::new(ReleaseConfig::for_mock(&base, Some(r.key.public_text.clone())));
    let offline = Releases::new(Http::new(cfg).unwrap(), r.user.path().join("cache"));
    let c = offline.check_latest().await;
    assert_eq!(c.state, CheckState::Offline);
    assert_eq!(c.release.as_ref().unwrap().body, "cached notes");
    assert!(c.manifest.is_some());
    assert_eq!(c.checked_at, first.checked_at, "the time of the last successful check is kept");
    // and with no cache at all, offline is still just "offline"
    let empty = Releases::new(offline.http().clone(), r.user.path().join("other-cache"));
    let c2 = empty.check_latest().await;
    assert_eq!(c2.state, CheckState::Offline);
    assert!(c2.release.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_release_list_is_capped_at_20_and_hides_drafts_and_prereleases() {
    let r = rig();
    let mut arr = Vec::new();
    for i in 0..25 {
        arr.push(serde_json::json!({"tag_name": format!("v0.{i}.0"), "name": format!("r{i}"), "body": "", "draft": false, "prerelease": false, "assets": []}));
    }
    arr.push(serde_json::json!({"tag_name": "v9.0.0", "draft": true, "assets": []}));
    arr.push(serde_json::json!({"tag_name": "v9.1.0", "prerelease": true, "assets": []}));
    arr.push(serde_json::json!({"tag_name": "junk", "assets": []}));
    r.mock.set(&format!("/repos/{REPO}/releases?per_page=20"), Route::Ok { body: serde_json::to_vec(&arr).unwrap(), etag: None });
    let list = r.releases.list(20).await.unwrap();
    assert_eq!(list.len(), 20);
    assert_eq!(list[0].tag, "v0.24.0", "newest first, 0.24 > 0.9 numerically");
    assert!(list.iter().all(|x| !x.tag.starts_with("v9") && x.tag != "junk"));
    assert_eq!(r.releases.list(5).await.unwrap().len(), 5);
    r.mock.set(&format!("/repos/{REPO}/releases?per_page=20"), Route::Ok { body: b"nope".to_vec(), etag: None });
    assert_eq!(r.releases.list(20).await.unwrap_err().kind, Kind::BadResponse);
}

#[tokio::test(flavor = "multi_thread")]
async fn assets_on_foreign_hosts_are_dropped() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    let mut v: serde_json::Value = serde_json::from_str(&fx.release_json).unwrap();
    v["assets"][1]["browser_download_url"] = "https://evil.example/game-0.2.0.zip".into();
    v["assets"][2]["browser_download_url"] = "http://github.com/OWNER/rusty-mcla/releases/download/v0.2.0/RustyMCLA.exe".into();
    r.mock.set(&fx.paths.latest, Route::Ok { body: v.to_string().into_bytes(), etag: None });
    r.mock.set(&fx.paths.manifest, Route::Ok { body: fx.manifest_json.clone().into_bytes(), etag: None });
    let c = r.releases.check_latest().await;
    let rel = c.release.unwrap();
    assert!(rel.asset("manifest.json").is_some());
    assert!(rel.asset("game-0.2.0.zip").is_none() && rel.asset("RustyMCLA.exe").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_to_a_host_that_is_not_allowed_is_refused() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    fx.publish(&r.mock, "\"e1\"");
    let c = r.releases.check_latest().await;
    let (rel, m) = (c.release.unwrap(), c.manifest.unwrap());
    r.mock.set(&fx.paths.zip, Route::Redirect("http://evil.example/game.zip".into()));
    old_game(r.root.path());
    let e = package::download_and_install(&r.http, &rel, &m, r.root.path(), r.user.path(), &Options::default(), &Cancel::new(), &mcla_updater::no_progress()).await.unwrap_err();
    assert!(matches!(e.kind, Kind::Policy | Kind::Offline), "{e}");
    assert!(leftovers(r.root.path()).is_empty(), "{:?}", leftovers(r.root.path()));
    assert_eq!(std::fs::read(r.root.path().join("game").join("mcla.exe")).unwrap(), b"OLD GAME");
    // a redirect inside the allowed host works
    let target = format!("{}/mirror/game.zip", r.mock.base);
    r.mock.set("/mirror/game.zip", Route::Ok { body: fx.zip.clone(), etag: None });
    r.mock.set(&fx.paths.zip, Route::Redirect(target));
    package::download_and_install(&r.http, &rel, &m, r.root.path(), r.user.path(), &Options::default(), &Cancel::new(), &mcla_updater::no_progress()).await.unwrap();
    assert_eq!(std::fs::read(r.root.path().join("game").join("mcla.exe")).unwrap(), b"MZ game 0.2.0");
}

#[tokio::test(flavor = "multi_thread")]
async fn game_update_end_to_end_swaps_keeps_game_old_and_backs_up_the_save() {
    let r = rig();
    old_game(r.root.path());
    save(r.user.path());
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "## Fixed\n- all");
    fx.publish(&r.mock, "\"e1\"");
    let c = r.releases.check_latest().await;
    let (rel, m) = (c.release.unwrap(), c.manifest.unwrap());

    let installed = Installed { launcher: "0.1.0".into(), game_version: Some("0.1.0".into()), game_data_version: Some(1), built_data_version: Some(1) };
    let p = plan(&m, &installed, None);
    assert!(p.game_update && p.launcher_update && p.offer && !p.update_needs_rebuild);

    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let progress: mcla_updater::ProgressFn = Arc::new(move |p| s2.lock().unwrap().push((p.phase, p.done, p.total)));
    let rec = package::download_and_install(&r.http, &rel, &m, r.root.path(), r.user.path(), &Options::default(), &Cancel::new(), &progress).await.unwrap();
    assert_eq!(rec.version, "0.2.0");
    assert_eq!(rec.size, fx.zip.len() as u64);
    assert_eq!(std::fs::read(r.root.path().join("game").join("mcla.exe")).unwrap(), b"MZ game 0.2.0");
    assert_eq!(std::fs::read(r.root.path().join("game.old").join("mcla.exe")).unwrap(), b"OLD GAME");
    assert!(r.root.path().join("game").join("data").join("shaders.bin").is_file());
    assert!(!r.root.path().join(".update").exists() && !r.root.path().join("game.new").exists());
    let backups: Vec<_> = std::fs::read_dir(r.user.path().join("backups")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert!(backups[0].ends_with("-before-update.json"));
    assert_eq!(std::fs::read_to_string(r.user.path().join("saves").join("mcla_save.json")).unwrap(), "{\"career\":1}");
    let phases: Vec<_> = seen.lock().unwrap().iter().map(|x| x.0).collect();
    assert!(phases.contains(&mcla_updater::Phase::DownloadGame) && phases.contains(&mcla_updater::Phase::SwapGame));
    let last_dl = seen.lock().unwrap().iter().filter(|x| x.0 == mcla_updater::Phase::DownloadGame).last().cloned().unwrap();
    assert_eq!((last_dl.1, last_dl.2), (fx.zip.len() as u64, fx.zip.len() as u64));
    // what is installed is now recorded, and equals the release
    let on_disk = package::read_record(r.root.path()).unwrap();
    assert_eq!(on_disk.sha256, fx.manifest.game.sha256);
    let after = plan(&m, &Installed { game_version: Some(on_disk.version), game_data_version: Some(on_disk.data_version), ..installed }, None);
    assert!(!after.game_update);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_hash_is_refused_and_everything_stays_as_it_was() {
    let r = rig();
    old_game(r.root.path());
    save(r.user.path());
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    fx.publish(&r.mock, "\"e1\"");
    // the server delivers a different (same-length) file than the manifest describes
    let mut tampered = fx.zip.clone();
    let n = tampered.len() / 2;
    tampered[n] ^= 0xff;
    r.mock.set(&fx.paths.zip, Route::Ok { body: tampered, etag: None });
    let c = r.releases.check_latest().await;
    let (rel, m) = (c.release.unwrap(), c.manifest.unwrap());
    let e = package::download_and_install(&r.http, &rel, &m, r.root.path(), r.user.path(), &Options::default(), &Cancel::new(), &mcla_updater::no_progress()).await.unwrap_err();
    assert_eq!(e.kind, Kind::Hash, "{e}");
    assert_eq!(std::fs::read(r.root.path().join("game").join("mcla.exe")).unwrap(), b"OLD GAME");
    assert!(leftovers(r.root.path()).is_empty(), "{:?}", leftovers(r.root.path()));
    assert!(!r.user.path().join("backups").exists(), "the save is only touched once the package is verified");
}

#[tokio::test(flavor = "multi_thread")]
async fn truncated_and_padded_downloads_are_refused() {
    let r = rig();
    old_game(r.root.path());
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    fx.publish(&r.mock, "\"e1\"");
    let c = r.releases.check_latest().await;
    let (rel, m) = (c.release.unwrap(), c.manifest.unwrap());
    let run = |r: &Rig, rel: &mcla_updater::releases::Release, m: &mcla_updater::manifest::ReleaseManifest| {
        let (http, root, user, rel, m) = (r.http.clone(), r.root.path().to_path_buf(), r.user.path().to_path_buf(), rel.clone(), m.clone());
        async move { package::download_and_install(&http, &rel, &m, &root, &user, &Options::default(), &Cancel::new(), &mcla_updater::no_progress()).await }
    };

    // (a) a short file on the server: Content-Length matches what it sends, but not the manifest
    r.mock.set(&fx.paths.zip, Route::Ok { body: fx.zip[..fx.zip.len() / 2].to_vec(), etag: None });
    assert_eq!(run(&r, &rel, &m).await.unwrap_err().kind, Kind::Size);
    // (b) the connection dies half way (announced length = manifest size)
    r.mock.set(&fx.paths.zip, Route::Cut { declared: fx.zip.len(), sent: fx.zip[..fx.zip.len() / 2].to_vec() });
    let e = run(&r, &rel, &m).await.unwrap_err();
    assert!(matches!(e.kind, Kind::Offline | Kind::Size), "{e}");
    // (c) more bytes than the manifest says
    let mut longer = fx.zip.clone();
    longer.extend_from_slice(b"extra");
    r.mock.set(&fx.paths.zip, Route::Ok { body: longer, etag: None });
    assert_eq!(run(&r, &rel, &m).await.unwrap_err().kind, Kind::Size);

    assert_eq!(std::fs::read(r.root.path().join("game").join("mcla.exe")).unwrap(), b"OLD GAME");
    assert!(leftovers(r.root.path()).is_empty(), "{:?}", leftovers(r.root.path()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_download_leaves_nothing_behind() {
    let r = rig();
    old_game(r.root.path());
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    fx.publish(&r.mock, "\"e1\"");
    let c = r.releases.check_latest().await;
    let (rel, m) = (c.release.unwrap(), c.manifest.unwrap());
    let cancel = Cancel::new();
    cancel.cancel();
    let e = package::download_and_install(&r.http, &rel, &m, r.root.path(), r.user.path(), &Options::default(), &cancel, &mcla_updater::no_progress()).await.unwrap_err();
    assert!(e.is_cancelled());
    assert!(leftovers(r.root.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malicious_zip_with_a_good_hash_is_still_refused_by_the_extractor() {
    let r = rig();
    old_game(r.root.path());
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    // the manifest honestly describes a zip that tries to escape its folder
    let evil = make_zip(&[("mcla.exe", b"MZ"), ("../../escaped.txt", b"gotcha"), ("pad.bin", &noise(3000, 1))]);
    let mut v: serde_json::Value = serde_json::to_value(&fx.manifest).unwrap();
    v["game"]["size"] = evil.len().into();
    v["game"]["sha256"] = sha(&evil).into();
    let mjson = v.to_string();
    let mut rel_v: serde_json::Value = serde_json::from_str(&fx.release_json).unwrap();
    rel_v["assets"][1]["size"] = evil.len().into();
    r.mock.set(&fx.paths.latest, Route::Ok { body: rel_v.to_string().into_bytes(), etag: Some("\"e\"".into()) });
    r.mock.set(&fx.paths.manifest, Route::Ok { body: mjson.into_bytes(), etag: None });
    r.mock.set(&fx.paths.zip, Route::Ok { body: evil, etag: None });
    let c = r.releases.check_latest().await;
    let (rel, m) = (c.release.unwrap(), c.manifest.unwrap());
    let e = package::download_and_install(&r.http, &rel, &m, r.root.path(), r.user.path(), &Options::default(), &Cancel::new(), &mcla_updater::no_progress()).await.unwrap_err();
    assert_eq!(e.kind, Kind::Zip, "{e}");
    assert!(!r.root.path().parent().unwrap().join("escaped.txt").exists());
    assert_eq!(std::fs::read(r.root.path().join("game").join("mcla.exe")).unwrap(), b"OLD GAME");
    assert!(leftovers(r.root.path()).is_empty());
}

fn fake_exe(root: &std::path::Path) -> PathBuf {
    let exe = root.join("RustyMCLA.exe");
    std::fs::write(&exe, "OLD LAUNCHER").unwrap();
    exe
}

#[tokio::test(flavor = "multi_thread")]
async fn launcher_self_update_verifies_hash_and_signature_then_swaps() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    fx.publish(&r.mock, "\"e1\"");
    let c = r.releases.check_latest().await;
    let (rel, m) = (c.release.unwrap(), c.manifest.unwrap());
    let exe = fake_exe(r.root.path());

    let staged = selfupdate::download_and_stage(&r.http, &rel, &m, &exe, &Cancel::new(), &mcla_updater::no_progress()).await.unwrap();
    assert_eq!(std::fs::read(&staged).unwrap(), fx.launcher);
    assert_eq!(std::fs::read(&exe).unwrap(), b"OLD LAUNCHER", "the running exe is untouched until apply");
    let old = selfupdate::apply(&exe, &staged).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), fx.launcher);
    assert_eq!(std::fs::read(&old).unwrap(), b"OLD LAUNCHER");
    assert!(selfupdate::cleanup_after_start(&exe));
    assert!(!old.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn self_update_refuses_a_file_signed_by_another_key_and_a_missing_key() {
    let r = rig();
    let attacker = new_key();
    // the release is internally consistent (hash matches) but signed with a key the launcher does not trust
    let fx = fixture(&r.mock.base, "0.2.0", &attacker, 1, "n");
    fx.publish(&r.mock, "\"e1\"");
    let c = r.releases.check_latest().await;
    let (rel, m) = (c.release.unwrap(), c.manifest.unwrap());
    let exe = fake_exe(r.root.path());
    let e = selfupdate::download_and_stage(&r.http, &rel, &m, &exe, &Cancel::new(), &mcla_updater::no_progress()).await.unwrap_err();
    assert_eq!(e.kind, Kind::Signature, "{e}");
    assert_eq!(std::fs::read(&exe).unwrap(), b"OLD LAUNCHER");
    assert!(!selfupdate::part_path(&exe).exists() && !selfupdate::new_path(&exe).exists());

    // a build with no key set refuses before downloading anything
    let cfg = Arc::new(ReleaseConfig::embedded("0.1.0"));
    let mut cfg2 = (*cfg).clone();
    cfg2.api_base = r.mock.base.clone();
    cfg2.web_base = r.mock.base.clone();
    cfg2.mock_authority = Some(r.mock.base.trim_start_matches("http://").to_string());
    // the shipped update-key.pub is now the real key: simulate a build whose key file is still the placeholder
    cfg2.public_key = "untrusted comment: NOT SET. Replace this whole file with the public key made offline by `minisign -G`.\n".to_string();
    let http = Http::new(Arc::new(cfg2)).unwrap();
    r.mock.clear_hits();
    let e = selfupdate::download_and_stage(&http, &rel, &m, &exe, &Cancel::new(), &mcla_updater::no_progress()).await.unwrap_err();
    assert_eq!(e.kind, Kind::Signature);
    assert!(e.detail.contains("no signing key"));
    assert_eq!(r.mock.total_hits(), 0, "nothing was downloaded");
}

#[tokio::test(flavor = "multi_thread")]
async fn self_update_refuses_a_truncated_or_tampered_launcher() {
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    fx.publish(&r.mock, "\"e1\"");
    let c = r.releases.check_latest().await;
    let (rel, m) = (c.release.unwrap(), c.manifest.unwrap());
    let exe = fake_exe(r.root.path());
    let mut tampered = fx.launcher.clone();
    tampered[1000] ^= 1;
    r.mock.set(&fx.paths.launcher, Route::Ok { body: tampered, etag: None });
    assert_eq!(selfupdate::download_and_stage(&r.http, &rel, &m, &exe, &Cancel::new(), &mcla_updater::no_progress()).await.unwrap_err().kind, Kind::Hash);
    r.mock.set(&fx.paths.launcher, Route::Ok { body: fx.launcher[..1000].to_vec(), etag: None });
    assert_eq!(selfupdate::download_and_stage(&r.http, &rel, &m, &exe, &Cancel::new(), &mcla_updater::no_progress()).await.unwrap_err().kind, Kind::Size);
    assert_eq!(std::fs::read(&exe).unwrap(), b"OLD LAUNCHER");
    assert!(!selfupdate::part_path(&exe).exists() && !selfupdate::new_path(&exe).exists() && !selfupdate::old_path(&exe).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn install_a_specific_older_version_from_the_list() {
    let r = rig();
    old_game(r.root.path());
    let newest = fixture(&r.mock.base, "0.3.0", &r.key, 1, "newest");
    let older = fixture(&r.mock.base, "0.2.0", &r.key, 1, "older");
    newest.publish(&r.mock, "\"e3\"");
    older.publish(&r.mock, "\"e2\"");
    r.mock.set(&format!("/repos/{REPO}/releases?per_page=20"), Route::Ok { body: format!("[{},{}]", newest.release_json, older.release_json).into_bytes(), etag: None });
    let list = r.releases.list(20).await.unwrap();
    assert_eq!(list.iter().map(|x| x.tag.as_str()).collect::<Vec<_>>(), ["v0.3.0", "v0.2.0"]);
    let pick = &list[1];
    let m = r.releases.manifest_for(pick).await.unwrap();
    assert_eq!(m.version, "0.2.0");
    package::download_and_install(&r.http, pick, &m, r.root.path(), r.user.path(), &Options::default(), &Cancel::new(), &mcla_updater::no_progress()).await.unwrap();
    assert_eq!(std::fs::read(r.root.path().join("game").join("mcla.exe")).unwrap(), b"MZ game 0.2.0");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rollback_to_an_old_signed_launcher_is_refused() {
    let r = rig();
    // a genuine old build (signed for 0.1.0 by the real key) offered by a manifest that claims 0.2.0
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "n");
    let mut v: serde_json::Value = serde_json::to_value(&fx.manifest).unwrap();
    v["launcher"]["signature"] = r.key.sign(&fx.launcher, "0.1.0").into();
    fx.publish(&r.mock, "\"e1\"");
    r.mock.set(&fx.paths.manifest, Route::Ok { body: v.to_string().into_bytes(), etag: None });
    let c = r.releases.check_latest().await;
    let (rel, m) = (c.release.unwrap(), c.manifest.unwrap());
    let exe = fake_exe(r.root.path());
    let e = selfupdate::download_and_stage(&r.http, &rel, &m, &exe, &Cancel::new(), &mcla_updater::no_progress()).await.unwrap_err();
    assert_eq!(e.kind, Kind::Signature);
    assert!(e.detail.contains("RustyMCLA 0.1.0"), "{e}");
    assert_eq!(std::fs::read(&exe).unwrap(), b"OLD LAUNCHER");
    assert!(!selfupdate::part_path(&exe).exists() && !selfupdate::new_path(&exe).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blocked_api_falls_back_to_the_manifest_on_github_com() {
    // GitHub #20: api.github.com cannot be reached, github.com can: the latest release comes from its manifest.json alone
    let r = rig();
    let fx = fixture(&r.mock.base, "0.2.0", &r.key, 1, "notes");
    r.mock.set(&fx.paths.latest, Route::Status { code: 502, headers: vec![], body: vec![] });
    r.mock.set(&format!("/{REPO}/releases/latest/download/manifest.json"), Route::Ok { body: fx.manifest_json.clone().into_bytes(), etag: None });
    let c = r.releases.check_latest().await;
    assert_eq!(c.state, CheckState::Fresh);
    let rel = c.release.unwrap();
    assert_eq!(rel.tag, "v0.2.0");
    let zip = rel.asset("game-0.2.0.zip").expect("the game package is named by the manifest");
    assert_eq!(zip.url, format!("{}/{REPO}/releases/download/v0.2.0/game-0.2.0.zip", r.mock.base));
    assert_eq!(c.manifest.unwrap().game.version, "0.2.0");
    // the cache reads back as the same release
    assert_eq!(r.releases.cached().release.unwrap().tag, "v0.2.0");

    // a manifest that cannot be read is refused: the state stays what the API said
    r.mock.set(&format!("/{REPO}/releases/latest/download/manifest.json"), Route::Ok { body: br#"{"schema": 2}"#.to_vec(), etag: None });
    let d = tempfile::tempdir().unwrap();
    let fresh = Releases::new(r.http.clone(), d.path().join("cache"));
    assert!(matches!(fresh.check_latest().await.state, CheckState::Failed { .. }));
}
