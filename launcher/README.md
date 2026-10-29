# Rusty MCLA launcher

The launcher for the unofficial PC port (see `../docs/LAUNCHER.md` for the plan, the decisions and the visual design).
This folder is the **launcher shell**: window, tabs, themes, language support, settings and logs, plus the live **Install page** (disc validation, build with progress, Cancel / Resume, verify; the build stages are extract, translate shaders, audio, video, verify) on top of the
`mcla-install` crate (`../mcla-rs/crates/install`, used by path; MSVC and GNU both build it). The **update system** (release check, notes, game package, launcher self-update; library `updater/`, glue `src/update.rs`, page `ui/js/update.js`) is built, and so is the **Controls tab** (the game's own keyboard bindings, read from and written into the save through `mcla_config::keybinds`; `src/controls.rs`, `ui/js/controls.js`) with the live **Monitor** row in Settings > Display and game (`src/display.rs`); **Export Logs, the save manager and Play** are built too (library `support/`, glue `src/support.rs`, page `ui/js/support.js`); only CI is later work (listed in "Status and open list" of `../docs/LAUNCHER.md`).

Tauri 2 + a plain HTML/CSS/JS page. No npm, no bundler, no Tauri CLI: the only toolchain is Rust. It is its own cargo workspace,
**not** a member of the game's workspace (`../mcla-rs`, GNU toolchain). Licence: GPL-3.0-only (`../LICENSE`).
No game art, screenshots, audio or Rockstar / Take-Two branding anywhere in this folder; the hero picture is an original SVG scene.

## Build

The launcher is built with the **MSVC** toolchain and a **static CRT**, so the result is one exe that imports only Windows
system DLLs (the GNU build would need `WebView2Loader.dll` beside it). Needs the VS 2022 Build Tools and the
`stable-x86_64-pc-windows-msvc` toolchain; users of the exe need nothing but the WebView2 runtime (Windows 11 and current Windows 10 have it).

```powershell
$env:RUSTFLAGS = "-C target-feature=+crt-static"
cd launcher                                               # in your clone of the repository
cargo +stable-x86_64-pc-windows-msvc build --release      # -> target\release\rusty-mcla.exe  (about 6 MB)
cargo +stable-x86_64-pc-windows-msvc test --release       # settings (3), controls (2) and support glue (3) tests
cargo +stable-x86_64-pc-windows-msvc test --release -p mcla-updater    # update system + save backups: 72 unit + 21 integration tests against a local mock (about 30 s)
cargo +stable-x86_64-pc-windows-msvc test --release -p mcla-support    # redaction, system information, the logs zip, save manager (backup / restore / import, also saves of the original game), game start (35 tests)
cargo +stable-x86_64-pc-windows-msvc build --release -p mcla-updater --bins --examples   # mk-manifest, make_fixture, mock_server, static_server
```

While working, build into a private target (`$env:CARGO_TARGET_DIR = "<another folder>\launcher-target"`);
the final build uses the default `target` here. The page is embedded at compile time, so rebuild after editing `ui/`.

## Run and check

Run `rusty-mcla.exe` (rename it `RustyMCLA.exe` for release). It opens a frameless 1280 x 760 window. Nothing is installed or
downloaded: with no `manifest.local.json` beside the exe it shows the **first run** Home page.

- `RUSTY_MCLA_USER_DIR=<folder>` puts settings and logs there instead of `%LOCALAPPDATA%\RustyMCLA` (tests use this).
- `RUSTY_MCLA_INSTALL_TEST=<iso>` (with `RUSTY_MCLA_SELFTEST=<report>`, optionally `RUSTY_MCLA_INSTALL_TEST_CANCEL=1`): the page validates the disc, presses Start (cancels once and resumes), reports what it showed and exits. The data is built into the folder of the exe.
- Update system against a **mock** of the GitHub API (the real repository does not exist yet; details in `../docs/LAUNCHER.md`, "Update system built"):
  `make_fixture <dir> --base http://127.0.0.1:47653 --release 0.2.0 [--launcher-exe <exe>] [--older 0.1.0]` writes a mock release (needs `MOCK_GAME_MB=60` for a big package),
  `mock_server <dir> 47653 [--mode slow|ratelimit|malformed|tampered]` serves it, and the launcher is started with `RUSTY_MCLA_TEST_MOCK=http://127.0.0.1:47653` and
  `RUSTY_MCLA_TEST_PUBKEY=<dir>\key.pub` (the only way to change the release host or the signing key from the environment; loopback only). `RUSTY_MCLA_RELEASE_REPO=owner/name` points a build at a fork.
  `RUSTY_MCLA_UPDATE_TEST=check,skip,clear,update,setup,list:install:v0.1.0,local:<zip>,wait:500` (with `RUSTY_MCLA_SELFTEST=<report>`) drives the page like a user and reports what it showed.
  The PowerShell scripts that ran the mock and the real exe (`drive.ps1`, `alone.ps1`, `ut.ps1`) are kept in `../work/launcher-updates/`.
- `RUSTY_MCLA_CONTROLS_TEST=1` (with `RUSTY_MCLA_SELFTEST=<report>`): the page drives the Controls tab like a user (rebind, "already used" REPLACE / CANCEL, clear, Esc, reserved keys, Alt+Enter, must-be-mapped, reset) and the Monitor row against the save in `<user dir>\saves` and `settings.toml`, then reports. Use a scratch `RUSTY_MCLA_USER_DIR` with a **copy** of a save; the script that ran it is `../work/launcher-controls/ctest.ps1`.
- `RUSTY_MCLA_SUPPORT_TEST=<steps>` (with `RUSTY_MCLA_SELFTEST=<report>`): the page drives Export Logs, the save manager and Play like a user and reports what it showed. Steps: `export[:save+shots]`, `report`, `backup`, `restore:<n>`, `import:<path>` (a port save, an Xbox 360 `mc4.sav` or STFS package `CON ` / `LIVE` / `PIRS`, a PS3 folder / `RAGE.SAV`; needs a race table under `<root>\data\cache\tune\race\` for the last two), `play`, `playraw`, `crash`, `dismiss`, `goback`, `keep:<n>`, `updcheck`, `updstart`, `sample:<n>`, `refresh`, `wait:<ms>`; confirmations are accepted automatically; with `RUSTY_MCLA_SELFTEST` set PLAY never closes the launcher. Use a scratch `RUSTY_MCLA_USER_DIR` and the stand-in game (`cargo build --release -p mcla-support --example standin`; it records its command line in `STANDIN_OUT`, exits with `STANDIN_EXIT`, sleeps `STANDIN_SLEEP` s). The scripts that ran all of this (`sup.ps1`, `savetest.ps1`, `playtest.ps1`, `playclick.ps1`, `updplay.ps1`, `zipcheck.py`, `realgame.ps1`) are kept in `../work/launcher-support/`.
- `RUSTY_MCLA_MODS_TEST=<steps>` (with `RUSTY_MCLA_SELFTEST=<report>`): the page drives the Mods tab like a user and reports what it showed (rows, detail card, toasts, questions asked). Steps: `snapshot`, `answer:yes|no` (the older-version / remove questions; yes at first), `drop:<zip>[|<zip>...]` (as if dropped on the window), `select:<n>`, `switch:<n>`, `switchbtn`, `remove:<n>`, `open`, `dialog:older|remove` (shows the real dialog, then cancels), `shot:<name>` (logs `MODS-SHOT <name>`), `settle[:ms]`, `wait:<ms>`, `reload`, `hover:on|off`. Needs scratch `RUSTY_MCLA_USER_DIR`, `MCLA_SETTINGS`, `MCLA_MODS_DIR` (the mods folder; the game's own test hook) and `MCLA_DATA` (the data folder, only read). The scripts (`prep.py`, `run.ps1`, `acts/`) are kept in `../work/mods/s5/`; `run.ps1` screenshots the window when it sees a `MODS-SHOT` line and never sends mouse input.
- `RUSTY_MCLA_SELFTEST=<file>` makes the page measure itself (every page's card rectangles and overflow, all theme tokens, language
  fallback, settings round trip through Rust, locked tabs unreachable, and `installFit`: the four Install-page cards in every state of the build - idle, each stage running with its sub-bar, cancelled, every error text, done, repair - in three themes, as free pixels under the last element and horizontal overflow; any card with less than 0 px or any overflow makes the report `ok: false`), write a JSON report and exit. `Install.fitStates()` is the same check, callable in a browser against the served `ui/`.
- Design check without the exe: serve `ui/` with any static server and open `index.html?dev=1&installed=1&page=<home|install|controls|settings|support|mods|music-preview>&sub=<...>&theme=<id>` (add `&mods=sample` to see the Mods page with one mod of every state)
  in a browser (without Tauri the page uses defaults; the preview pages of the locked tabs are only reachable this way).

## Layout

```
Cargo.toml  build.rs  tauri.conf.json      own workspace; frameless 1280x760, fixed, centred; frontendDist ./ui
src/main.rs        window commands (minimise, close, drag), boot, settings, open folder, self-test hook
src/platform.rs    everything per operating system (data dir, portable root, open a folder); no cfg() elsewhere
src/install.rs     Tauri commands of the Install page (info, file picker, validate, start / cancel, poll, verify) over `mcla-install`; build log `build-*.log`
src/controls.rs    Tauri commands of the Controls tab's keyboard table over `mcla_config::keybinds` (read, set with the "already used" check, clear, reset; writes only the save's `keyboard` member, `.bak` once per session; never creates the save)
src/display.rs     Settings > Display and game: the monitor list (0 = primary, then the system's order, as the game numbers them) and `display.monitor` via `mcla_config::set_key`
src/support.rs     Tauri glue of the support features over `mcla-support`: Export Logs, the save manager, Play (watch, crash notice), go back to the previous game version, the system card
support/           crate `mcla-support` (no Tauri): `redact` (user / machine name), `sysinfo`, `export` (the logs zip, allow-list), `saves` (status, backup, restore, import of a port save or conversion of an original-game save through `mcla-savedecode`, with `inspect` for the confirmation text), `launch` (command line, watch, `last_run.json`), `examples/standin.rs`
src/update.rs      Tauri commands of the update system (status poll, check, update now, game setup, release list, specific version, local package, open_url) over `mcla-updater`; the run state
updater/           crate `mcla-updater` (no Tauri): manifest.json + validator, GitHub Releases client + cache, notes renderer, plan, game package, self-update, save backup;
                   `src/bin/mk_manifest.rs` (makes a release's manifest), `tests/` (mock server + integration tests), `examples/` (mock GitHub server, fixture builder, static server)
release.json       the repository (`holdmysocks/rusty-mcla`, public), API base, host allow-list: the one place to change it
update-key.pub     the minisign public key of the launcher signature (placeholder until the real key exists)      manifest.example.json  a sample release manifest
src/settings.rs    %LOCALAPPDATA%\RustyMCLA\launcher.json: versioned, unknown keys kept, atomic write, bad file moved aside
src/logging.rs     %LOCALAPPDATA%\RustyMCLA\logs\launcher-<time>.log, newest 10 kept
icons/             icon.ico (16/32/48/256) and icon.png (Linux needs the PNG); original art, `work\launcher\make-icon.ps1` regenerates
ui/index.html      shell, icon sprite, hero art, every page (Home, First run, Install, Controls, Support, Settings, Mods, locked Music preview)
ui/css/themes.css  the eight themes as CSS custom properties, switched by <html data-theme="...">
ui/css/launcher.css  layout and components (cards place themselves with --x --y --w --h on the 1280x760 grid)
ui/js/themes.js    theme ids and apply()      ui/js/i18n.js  loader with per-key fallback to English
ui/js/bridge.js    the only code that calls Rust ui/js/app.js   tabs, sub-tabs, settings binding, theme picker, self-test
ui/js/install.js   Install page and first-run numbers: polls `install_poll`, draws checklist / progress / log; test hook RUSTY_MCLA_INSTALL_TEST
ui/js/controls.js  Controls > Keyboard (table, key capture, question dialog, status) and the Monitor row; test hook RUSTY_MCLA_CONTROLS_TEST; without Tauri `?kbd=ok` shows the table editable-looking, else read-only
ui/js/support.js   Export Logs, save manager (Support tab and the Home card), PLAY and its banner (failed start, crash notice), go back to the previous version; test hook RUSTY_MCLA_SUPPORT_TEST
ui/js/update.js    update banner, release notes, Updates card, Settings rows, release list dialog, Install step 1 (game package); test hook RUSTY_MCLA_UPDATE_TEST; dev mock states `?upd=available|downloading|stopped|offline|rate|rebuild|rebuildwarn|done|latest` (without Tauri only)
ui/locales/        en.json (every visible string, by key) and languages.json (the list the language picker shows)
```

Rules for the page: no sentences in code (add a key to `en.json`); colours only from the theme tokens; a button whose feature is not
built carries `data-later` (it shows a toast instead of doing nothing); anything that comes from a build or an install is a bracketed
placeholder (`[0.0.0]`, `[date and time]`) until the real value is wired in. The content security policy (`tauri.conf.json`) allows no
network connection from the page.

## Adding a language

Copy `ui/locales/en.json` to `ui/locales/<code>.json`, translate the values you want (missing keys fall back to English one by one),
and add `{ "code": "<code>", "name": "<native name>" }` to `ui/locales/languages.json`. The first start uses the system language when it is listed.

## Adding a theme

Add a `[data-theme="<id>"]` block to `ui/css/themes.css`, an entry to `ui/js/themes.js`, and `theme.<id>.name` / `.desc` keys to the locales.
