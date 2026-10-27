# mcla-rs - the Rust workspace

Project overview, status and what is next: `../docs/PROJECT.md` (the root `../README.md` is the short public page). Per-topic documentation: `../docs/` (index: `../docs/README.md`).
This file is only the guide to the workspace itself.

## Layout

| Path | What |
|---|---|
| `Cargo.toml` | Workspace: `crates/formats`, `crates/flash`, `crates/tool`, `crates/game`, `crates/config`, `crates/install`, `crates/savedecode`, `crates/shaders` (plus whatever other sessions add) |
| `crates/formats` (`mcla-formats`) | File formats: XGD2 disc image, RPF3 archives, RSC5 resources (LZX), DDS/JPEG, scripts (`sco`), string tables, AVM1, and the RAGE resource parsers under `rage/` |
| `crates/flash` (`mcla-flash`) | Player for the game's Flash movies (display list, timeline, triangle batches, software rasteriser for headless checks) |
| `crates/tool` (`mcla-tool`) | Inspection / extraction CLI (disc, archives, resources, shaders, scripts, swf, cars, navigation...); `install` and `ffmpeg` run the install pipeline, `registry` (maintainers' work tracking), `shaders-wgsl <data cache folder> [--iso <iso>]` runs only its "Translate shaders" stage on an existing extraction |
| `crates/install` (`mcla-install`) | The install pipeline as a library with no UI code: ISO validation (fast checks + SHA-256), extraction, ffmpeg lookup / pinned download, audio and video conversion (ports of `convert_audio.py` / `convert_video.py`), translates the original's sky / road / sea shader programs to WGSL (`shaders.rs`, over `mcla-shaders`), manifest, verify / repair; progress events and a cancel flag. Builds with GNU and MSVC (the launcher uses it by path) |
| `crates/savedecode` (`mcla-savedecode`) | Decoder of a save of the original game (Xbox 360 / Xenia `mc4.sav`, PS3 `RAGE.SAV`, also decrypted from its save folder with `PARAM.PFD`) into the port's save: `convert(bytes, current save, race table)`. A Rust port of `tools/import_mc4_save.py`, `tools/mc4save/` and `tools/ps3dec.py` (those stay as the reference; `tests/` compares the two on the real saves). No UI code; builds with GNU and MSVC (the launcher's IMPORT A SAVE uses it by path) |
| `crates/shaders` (`mcla-shaders`) | The translator of the original's Xbox 360 shader microcode (the Skyhat sky, the road and night road, the ocean, the cloud noise pass) to WGSL, a Rust port of `tools/xsh.py` + `tools/xs2wgsl.py` (byte-identical output). The eight `xs_*.wgsl` files it writes are a derivative of the game's data and are **not in this repository**: the install stage "Translate shaders" makes them on the player's machine from the player's own disc into `<data>/shaders_wgsl/`, and the game reads them from there at start-up (`crates/game/src/render/xs.rs`). Pure std + `regex`, builds with GNU and MSVC |
| `crates/game` (`mcla`, `mcla.exe`) | The game: wgpu renderer, physics under `physics/`, shaders under `render/`, all gameplay modules. `hooks.rs` holds the launcher hooks (settings, log, crash report, F12) |
| `crates/config` (`mcla-config`) | The `settings.toml` model, the key table (`schema()`, which keys the game implements), the data-folder resolution, `set_key` (one key written atomically), and the keyboard-binding model (`keybinds`: the 32 actions, defaults, reserved keys, conflicts, the save's `keyboard` member and an editor that changes only that member); no game dependencies, no winit, builds with GNU and MSVC (the launcher uses it by path; `crates/game/src/keybinds.rs` is the game's winit adapter) |
| `tools/` | Python helpers: `xsh.py` (Xbox 360 shader microcode disassembler), `xs2wgsl.py` (both are the Python reference of `crates/shaders`), recomp readers (`recomp_*.py`, `gfx_trace.py`, `gpu_trace.py`, `guestmem.py`), vehicle reference models and comparers (`*_reference.py`, `veh_*.py`), save tools (`import_mc4_save.py`, `ps3dec.py`, `mc4save/`), audio/video converters (`convert_audio.py`, `convert_video.py`) |
| `data/` | `tune_schema.txt`, `native_names.tsv`, `shader_param_names.json` |
| `names.txt` | Recovered archive entry names (for `mcla-tool`) |

Details of every crate, subcommand and tool: `../docs/FORMATS_TOOLS.md`.

## Build

Rust stable, GNU target on Windows (no Visual Studio needed). WinLibs mingw64 `bin` on `PATH` for `dlltool`.

```
cd mcla-rs                                               # in your clone of the repository
cargo build --release -p mcla
```

The final build is `target\release\mcla.exe`. Set `CARGO_TARGET_DIR` to build somewhere else (for example when several copies are built at once).

## Build the data (once)

```
mcla-tool install "<path to>.iso" <folder>     # -> <folder>/data/{cache,audio_pc,video_pc}, <folder>/manifest.local.json; about 2 minutes
mcla --data <folder>/data/cache
```

One command, no Python: validates the disc, extracts the archive, finds ffmpeg (`<folder>/tools/ffmpeg`, `--ffmpeg <path>`, `PATH`, otherwise downloads the pinned build and checks
its SHA-256), converts audio and video, translates the sky / road / sea shader programs from the disc (the game cannot draw those without them), writes the manifest, verifies. Re-running resumes; `--repair` re-checks and rebuilds what is missing. The same code (crate `mcla-install`)
runs behind the launcher's Install page. The older recipe still works and gives the same files:

```
mcla-tool extract "<path to>.iso" xarchive_cache.rpf <out>/cache names.txt
python tools/convert_audio.py <out> <out>/audio_pc     # kept for reference, superseded by `install`
python tools/convert_video.py "<iso>" <out>/video_pc                 # kept for reference, superseded by `install`
```

## Run

```
mcla                                   # data: see "Data folder" below
mcla --data <dir>                      # other data folder (a bare folder argument does the same)
mcla --settings <file>                 # other settings.toml (default %LOCALAPPDATA%\RustyMCLA\settings.toml; MCLA_SETTINGS=<file> too)
mcla --save-dir <dir>                  # other save folder (default %LOCALAPPDATA%\RustyMCLA\saves; MCLA_SAVE_FILE=<file> for one save file)
mcla --car <dir of a vp_* car>         # start with another car
mcla --fly                             # free-fly camera only
mcla --race-name <substr>              # start a race by name
mcla --screenshot out.png --frames 500 --pos x,y,z --yaw 0 --time 14 [--weather nice|cloudy|foggy|stormy]   # headless capture
```

**Data folder**, first match wins: `--data <dir>`, env `MCLA_DATA`, `paths.data_dir` of settings.toml (a relative path is taken from the install root = the exe's parent folder),
`<exe dir>/../data/cache`, `<exe dir>/data/cache`, then the developer fallback `D:/MCLA/extracted/cache` if it exists (TODO-remove-before-release). `audio_pc`, `video_pc` and the photo album
are looked up beside the data folder (`<data>/../audio_pc` ...), so the portable layout `data/{cache,audio_pc,music,video_pc}` works unchanged.

**Settings** (`settings.toml`, flags win over the file; a headless `--screenshot` run ignores the user's file unless `--settings` / `MCLA_SETTINGS` is given, and keeps no vsync):
`display.window_mode` (windowed / borderless / fullscreen; **Alt+Enter** cycles windowed -> borderless -> fullscreen in the game and writes the new value back), `display.resolution` (`"1600x900"`, physical pixels; fullscreen asks the chosen monitor for that video mode), `display.monitor` (0 = primary, then the others in the system's order; a missing one = primary with a warning), `display.vsync`,
`display.fps_cap` (0 = none), `game.intro` (normal / fast = no opening film / skip = no film and no arrival scene), `paths.data_dir`, `paths.screenshots`. The other keys
(aspect, upscaler, render_scale, aa) are read and kept but do nothing yet. The full table: `mcla_config::schema()`; details in `../docs/LAUNCHER.md`.

**Log and crashes**: a normal run writes `%LOCALAPPDATA%\RustyMCLA\logs\game-<yyyymmdd-hhmmss>.log` (newest 10 kept) besides stderr; a panic writes `crash-<time>.txt` beside it and the process exits with 101
(0 = normal exit, 1 = an error was returned). Headless `--screenshot` runs and the self-tests write no file. `RUSTY_MCLA_USER_DIR=<dir>` moves that whole folder (logs, saves, settings) for tests.
Test hooks: `MCLA_TEST_PANIC=main|thread|quit` (at frame 30), `MCLA_SHOT_KEY_TEST=<frame>` (presses F12 at that frame), `MCLA_ALT_ENTER_TEST=<frame>[,<frame>...]` (feeds Alt down, Enter, Enter repeat, Enter up, Alt up, plain Enter through the window's Alt+Enter handling at each frame and logs the window 30 frames later).

Do not start the exe with `--help` or unknown flags (it boots the game on the user's real save); always pass
`--screenshot` for captures. Debug switches are `MCLA_*` environment variables, documented in the topic docs
(for example `MCLA_DEBUG_VIEW`, `MCLA_NO_ENV`, `MCLA_NO_DRESS`, `MCLA_DAMAGE`).

Keyboard defaults (checked against `mcla_config::keybinds`, the model shared with the launcher's Controls tab; every gameplay key below is rebindable on the options page KEYBOARD):
`W`/`S` or arrows gas / brake-reverse, `A`/`D` or arrows steer, `Space` handbrake, `T` weight transfer, `Left Shift` nitrous, `E` special ability,
`Q` next ability, `F` headlights (flash), `H` horn, `V` change camera, `C` look back, `O` map, `M` or `Tab` GPS map, `N` next song, `B` music on/off,
`L` hydraulics (then `U` `I` `J` `K` the four wheels), `X`/`Y` bike lean back/forward, `G` go to garage, `R` reset vehicle. Fixed keys: `Esc` pause/menu,
`Enter` select, `Alt+Enter` window mode (windowed -> borderless -> fullscreen, saved to settings.toml), `Backspace` / `Delete` clear on the keyboard page, `F1` debug overlay, `F2` toggle free-fly camera, `F7` cycle weather, `[` `]` time of day, `F12` screenshot (PNG into the screenshots folder, `<exe dir>/../screenshots` by default).
Gamepad follows the original's default layout (`xenon_controls_trigger` in `tune/controller`) and its other controller styles; the full mapping,
the mapping editor and the keyboard page are in `../docs/CONTROLS_OPTIONS.md`.

## Tests

`cargo test --release` (the vehicle physics fixtures and save tests are in the `game` and `formats` crates);
the last full run is recorded in `../docs/reference/misc/baseline-tests.log` (early baseline) and in each topic doc's log.
