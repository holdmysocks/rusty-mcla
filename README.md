# Rusty MCLA

![Rust lines](https://img.shields.io/endpoint?url=https%3A%2F%2Fghloc.dev%2Fapi%2Fholdmysocks%2Frusty-mcla%2Fbadge%3Ffilter%3D.rs%24&label=Rust%20lines)
![License](https://img.shields.io/github/license/holdmysocks/rusty-mcla)
![Status](https://img.shields.io/badge/status-early%20development-orange)
![Platform](https://img.shields.io/badge/platform-Windows-blue)

An unofficial PC port of *Midnight Club: Los Angeles* (Complete Edition), written in Rust.

![The Rusty MCLA launcher, ready to play](assets/launcher-home.png)

It is a new game engine that reads the original game's own data from **your copy of the disc** and runs the game natively on PC
(Vulkan, DirectX 12 and Metal through wgpu). Nothing of the original game is included in this repository or in its releases.

> **Status: early development.** The game runs from data built from your disc image by the launcher. Early releases (currently 0.6.1) are
> published on the Releases page, but many things still differ from the original and bugs are expected.

## What you need

- **Your own copy of the game:** a disc image (`.iso`) of the Xbox 360 *Midnight Club: Los Angeles, Complete Edition (USA, Europe)*,
  legally obtained. This project does not provide it, does not help find it, and will not accept requests for it.
- Windows 10 or 11 (Linux and macOS are planned).
- A graphics card with Vulkan or DirectX 12 support, and enough free disk space for the game data built from your disc.

## How it works

1. Download the launcher from the Releases page. It is one portable file: put it in a folder of your choice.
2. Start it, choose your disc image, and let it build the game data into that folder. This is done once, on your computer.
3. Press Play. The launcher shows the latest release notes and offers updates, which you can always skip.

The launcher is built in this repository (`launcher/`): themes, language support, settings and logs, the disc setup, updates with release notes,
log export, a save manager and Play are in.

## What works today

Most of the game is in: the city, cars and the garage, the vehicle physics, the front end and menus, the minimap, career missions
and races, police and rivals, audio and cutscenes. Details that differ from the original, and what is still open, are tracked by the project.

The project's standard is 1:1 with the original, and every function of the original executable is tracked with a status (ported and verified, ported but not yet compared,
known but not ported, replaced by converted data, not needed). About 17 % of the functions are closed so far (verified against the original, recorded as a deliberate deviation, or not needed);
the rest is the work list: record the original, verify what is ported, port the gaps (opponent racers, vehicle physics, audio, police and lighting first).

Postponed for later: online play, custom music (including Spotify), car mods, controller rumble and adaptive triggers, racing wheels.

## Building from source

- The game and tools: `mcla-rs/` (Rust stable; the GNU target on Windows). See `mcla-rs/README.md`.
- The launcher: `launcher/` (Tauri 2; the MSVC toolchain with a static runtime, so it ships as a single exe). See `launcher/README.md`.

## Reporting problems

Open an issue. The launcher's Support tab can export a log file for you to attach (your Windows user name is removed first). There will be MANY bugs that will eventually be worked out as progress keeps getting made.
Please **do not post or ask for game files or disc images** in issues.

## Licence and notice

The code is licensed under the **GNU General Public License, version 3 only** (see `LICENSE`). Contributions are accepted under the same licence.
The licences of the third-party software compiled into the programs are listed in [`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md).

This is an unofficial fan project. It is not affiliated with, authorised or endorsed by Rockstar Games or Take-Two Interactive.
"Midnight Club" and "Midnight Club: Los Angeles" and related names and artwork belong to their owners and are used here only to say which game this
project works with. No game data, art, audio or video is distributed with it, and no game-specific keys: the only key material in the source is two public console constants (the Xbox 360 retail XEX key and the PS3 save-data key); the game's own key is derived on your machine from your own copy.
