// The only place that talks to Rust (Tauri commands). Without Tauri (the page opened in a plain browser, for design
// checks) every call falls back to harmless defaults so the layout can still be looked at.
(function () {
  const tauri = window.__TAURI__;
  const invoke = tauri && tauri.core && tauri.core.invoke ? tauri.core.invoke.bind(tauri.core) : null;
  const q = new URLSearchParams(location.search);

  const previewBoot = () => ({
    version: '0.1.0',
    os: 'windows',
    arch: 'x86_64',
    settings: { schema: 1, theme: q.get('theme') || 'steel', language: q.get('lang') || 'auto', open_on: 'home', check_updates: true, close_on_launch: true, backup_before_update: true, show_notes_after_update: true, remember_window: true, skipped_version: null, background: { rotate: true, interval_s: 10, folder: null } },
    installed: q.get('installed') === '1',
    folders: {},
    log_file: null,
    selftest: false,
    install_test: null,
    install_test_cancel: false,
    update_test: null,
  });

  let previewSettings = null;

  const previewSave = () => {
    const now = Math.floor(Date.now() / 1000);
    const b = (ago, reason, size) => ({ file: 'mcla_save-x-' + reason + '.json', unix: now - ago, reason, size });
    return { status: { exists: true, size: 504608, modified: now - 3600, path: '', game_running: false, keep: 10, backup_count: 4, last_backup: b(7200, 'manual', 504608) },
      backups: [b(7200, 'manual', 504608), b(86400 * 2, 'before-update', 503100), b(86400 * 3, 'before-restore', 498000), b(86400 * 9, 'before-import', 120400)] };
  };

  // design checks in a plain browser: the default table, read-only (as with no save)
  const PREVIEW_KEYS = [['STEER_LEFT', 'A', 'LEFT'], ['STEER_RIGHT', 'D', 'RIGHT'], ['GAS', 'W', 'UP'], ['BRAKE', 'S', 'DOWN'], ['SHIFT_UP'], ['SHIFT_DOWN'], ['HANDBRAKE', 'SPACE'], ['WEIGHT_XFER', 'T'], ['NITRO', 'LEFT SHIFT'], ['HEADLIGHTS', 'F'], ['HORN', 'H'], ['IN_ZONE', 'E'], ['ABILITY_NEXT', 'Q'], ['CAM_UP', 'V'], ['CAM_DOWN'], ['LOOKBACK', 'C'], ['MAP', 'O'], ['GPS_MAP', 'M', 'TAB'], ['MUSIC_UP', 'N'], ['MUSIC_DOWN'], ['MUSIC_PAUSE', 'B'], ['HYDRAULICS', 'L'], ['WHEEL_FRONT_LEFT', 'U'], ['WHEEL_FRONT_RIGHT', 'I'], ['WHEEL_BACK_LEFT', 'J'], ['WHEEL_BACK_RIGHT', 'K'], ['CONVERTIBLE_UP_DN'], ['LEAN_BACK', 'X'], ['LEAN_FORWARD', 'Y'], ['PAUSE'], ['GARAGE', 'G'], ['RESET_CAR', 'R']];
  const previewKbd = () => ({
    state: q.get('kbd') === 'ok' ? 'ok' : 'no_save', path: '', error: null, present: false, warning: null, defaults: true, unmapped: false,
    rows: PREVIEW_KEYS.map((r) => ({ id: r[0], keys: [r[1] ? { code: '', name: r[1] } : null, r[2] ? { code: '', name: r[2] } : null], must: false })),
    fixed: [['back', 'Esc'], ['accept', 'Enter'], ['window_mode', 'Alt+Enter'], ['clear', 'Backspace / Delete'], ['screenshot', 'F12'], ['developer', 'F1 F2 F7']],
  });

  window.Bridge = {
    available: !!invoke,
    boot: () => (invoke ? invoke('boot') : Promise.resolve(previewBoot())),
    saveSettings: (patch) => {
      if (invoke) return invoke('save_settings', { patch });
      previewSettings = previewSettings || previewBoot().settings;
      const merge = (d, p) => { for (const k in p) { if (p[k] && typeof p[k] === 'object' && !Array.isArray(p[k])) { d[k] = merge(d[k] || {}, p[k]); } else { d[k] = p[k]; } } return d; };
      return Promise.resolve(merge(previewSettings, patch));
    },
    log: (level, msg) => { if (invoke) invoke('log_line', { level, msg: String(msg) }).catch(() => {}); },
    openFolder: (id) => (invoke ? invoke('open_folder', { id }) : Promise.reject('missing')),
    minimise: () => { if (invoke) invoke('win_minimize'); },
    close: () => { if (invoke) invoke('win_close'); },
    drag: () => { if (invoke) invoke('win_drag'); },
    // install pipeline (Rust: src/install.rs). Without Tauri these do nothing, so the page can still be looked at in a browser.
    installInfo: () => (invoke ? invoke('install_info') : Promise.resolve(null)),
    pickIso: () => (invoke ? invoke('pick_iso') : Promise.resolve(null)),
    validateIso: (path) => (invoke ? invoke('validate_iso', { path }) : Promise.reject({ code: 'io', message: 'no bridge', detail: '' })),
    startInstall: (iso, ffmpegPath) => (invoke ? invoke('start_install', { iso, ffmpegPath: ffmpegPath || null }) : Promise.reject({ code: 'io', message: 'no bridge', detail: '' })),
    cancelInstall: () => (invoke ? invoke('cancel_install') : Promise.resolve()),
    installPoll: (since) => (invoke ? invoke('install_poll', { since }) : Promise.resolve(null)),
    verifyInstall: () => (invoke ? invoke('verify_install') : Promise.reject({ code: 'io', message: 'no bridge', detail: '' })),
    // update system (Rust: src/update.rs over the mcla-updater crate)
    updateStatus: () => (invoke ? invoke("update_status") : Promise.resolve(null)),
    updateCheck: (auto) => (invoke ? invoke("update_check", { auto: !!auto }) : Promise.resolve()),
    updateNow: () => (invoke ? invoke("update_now") : Promise.resolve()),
    updateSetup: (checkFirst) => (invoke ? invoke("update_setup", { checkFirst: !!checkFirst }) : Promise.resolve()),
    updateCancel: () => (invoke ? invoke("update_cancel") : Promise.resolve()),
    updateClearResult: () => (invoke ? invoke("update_clear_result") : Promise.resolve()),
    updateList: () => (invoke ? invoke("update_list") : Promise.reject({ code: "io", detail: "no bridge" })),
    updateInstallVersion: (tag) => (invoke ? invoke("update_install_version", { tag }) : Promise.resolve()),
    pickPackage: () => (invoke ? invoke("pick_package") : Promise.resolve(null)),
    updateUsePackage: (path) => (invoke ? invoke("update_use_package", { path }) : Promise.resolve()),
    openUrl: (url) => (invoke ? invoke("open_url", { url }) : Promise.reject("no bridge")),
    // Controls > Keyboard (src/controls.rs) and the monitor row (src/display.rs)
    kbdGet: () => (invoke ? invoke('kbd_get') : Promise.resolve(previewKbd())),
    kbdSet: (act, slot, code, alt, force) => (invoke ? invoke('kbd_set', { act, slot, code, alt: !!alt, force: !!force }) : Promise.resolve({ outcome: 'failed', error: { detail: 'no bridge' }, view: previewKbd() })),
    kbdClear: (act, slot) => (invoke ? invoke('kbd_clear', { act, slot }) : Promise.resolve({ outcome: 'failed', error: { detail: 'no bridge' }, view: previewKbd() })),
    kbdReset: () => (invoke ? invoke('kbd_reset') : Promise.resolve({ outcome: 'failed', error: { detail: 'no bridge' }, view: previewKbd() })),
    displayGet: () => (invoke ? invoke('display_get') : Promise.resolve({ monitors: [{ index: 0, name: 'DISPLAY1', width: 1920, height: 1080, primary: true }], monitor: 0, implemented: true, intro: 'normal', window_mode: 'windowed', resolution: '1600x900', resolutions: ['1280x720', '1366x768', '1600x900', '1920x1080'], vsync: true, fps_cap: 0, fps_caps: [0, 30, 60, 120, 144, 165, 240], fps_monitor: 'off', aa: 'original', texture_filtering: 'original', render_scale: 1, shadow_resolution: 'original', draw_distance: 'original', detail_level: 'original', bloom: true, dof: true, motion_blur: true, live_keys: ['game.intro', 'display.window_mode', 'display.resolution', 'display.vsync', 'display.fps_cap', 'display.fps_monitor', 'display.aa', 'display.bloom', 'display.dof', 'display.motion_blur', 'display.texture_filtering', 'display.render_scale', 'game.shadow_resolution', 'game.draw_distance', 'game.detail_level'], settings_path: '', file: 'missing' })),
    displaySetMonitor: (index) => (invoke ? invoke('display_set_monitor', { index }) : Promise.reject('no bridge')),
    displaySet: (key, value) => (invoke ? invoke('display_set', { key, value }) : Promise.reject('no bridge')),
    // support: Export Logs, save manager, Play (src/support.rs over mcla-support). Without Tauri they answer with a sample so the layout can be looked at.
    exportInfo: () => (invoke ? invoke('export_info') : Promise.resolve({ dir: 'C:\\Users\\you\\Desktop', default_dir: 'C:\\Users\\you\\Desktop', custom: false })),
    exportPickDir: () => (invoke ? invoke('export_pick_dir') : Promise.resolve(null)),
    exportSetDir: (dir) => (invoke ? invoke('export_set_dir', { dir }) : Promise.resolve()),
    exportLogs: (include) => (invoke ? invoke('export_logs', { include }) : Promise.reject({ code: 'io', detail: 'no bridge' })),
    exportReveal: () => (invoke ? invoke('export_reveal') : Promise.reject('no bridge')),
    reportUrl: () => (invoke ? invoke('report_url') : Promise.reject({ code: 'io', detail: 'no bridge' })),
    systemSummary: () => (invoke ? invoke('system_summary') : Promise.resolve({ launcher: 'v0.1.0', game: 'v0.2.0', windows: 'Windows 11 Pro 25H2 (build 26200.1), x86_64', graphics: 'GPU name, driver 1.2.3', webview: '155.0.0.0' })),
    systemText: () => (invoke ? invoke('system_text') : Promise.resolve('')),
    saveView: () => (invoke ? invoke('save_view') : Promise.resolve(previewSave())),
    saveBackupNow: () => (invoke ? invoke('save_backup_now') : Promise.reject({ code: 'io', detail: 'no bridge' })),
    saveRestore: (file, force) => (invoke ? invoke('save_restore', { file, force: !!force }) : Promise.reject({ code: 'io', detail: 'no bridge' })),
    saveImportPick: () => (invoke ? invoke('save_import_pick') : Promise.resolve(null)),
    saveImportInspect: (path) => (invoke ? invoke('save_import_inspect', { path }) : Promise.reject({ code: 'io', detail: 'no bridge' })),
    saveImport: (path, force) => (invoke ? invoke('save_import', { path, force: !!force }) : Promise.reject({ code: 'io', detail: 'no bridge' })),
    saveSetKeep: (n) => (invoke ? invoke('save_set_keep', { n }) : Promise.resolve(previewSave())),
    playInfo: () => (invoke ? invoke('play_info') : Promise.resolve({ installed: true, exe: true, updating: false, game_running: false, game_version: '0.2.0', close_on_launch: true, previous: q.get('prev') === '1', previous_version: '0.1.0' })),
    play: () => (invoke ? invoke('play') : Promise.reject({ code: 'io', detail: 'no bridge' })),
    crashNotice: () => (invoke ? invoke('crash_notice') : Promise.resolve(q.get('crash') === '1' ? { report: 'crash-20261015-130000.txt', code: 101, when: 0 } : null)),
    crashDismiss: () => (invoke ? invoke('crash_dismiss') : Promise.resolve()),
    goBackPrevious: (force) => (invoke ? invoke('go_back_previous', { force: !!force }) : Promise.reject({ code: 'io', detail: 'no bridge' })),
    selftestReport: (text) => (invoke ? invoke("selftest_report", { text }) : Promise.resolve()),
  };
})();
