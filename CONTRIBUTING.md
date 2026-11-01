# Contributing

Thanks for helping! Nock targets Windows 10/11 and is plain Rust against the `windows` crate.

## Build and check

```
cargo build --release
cargo test
cargo clippy
```

`cargo test` also checks that every scene script is well formed, that every window message number is unique, and that the games' basic rules work. The installer is built from `installer\nock.iss` with [Inno Setup 6](https://jrsoftware.org/isinfo.php) (`ISCC.exe /DMyAppVersion=1.2.3 installer\nock.iss`); CI builds it on every push and does a silent install/uninstall check.

## Linux

The Linux port shares the drawing API (`src/gfx.rs`), the games, the date helpers and the stores with Windows. Build and test it on Linux or in WSL:

```
sudo apt install build-essential fonts-dejavu-core
cargo test
NOCK_PREVIEW_DIR=/some/folder cargo test -- --ignored preview_pages preview_dropdowns
```

The second command draws every tab, the pill, the top bar and each dropdown to PNG files without opening a window. `packaging/linux/package.sh` makes the `.deb` and tarball; CI builds them and does an install and start-up smoke test.

- `src/gfx_linux.rs`: tiny-skia and fontdue behind the same `Gfx` API as `src/gfx_win.rs`.
- `src/linux_main.rs`: the notch, panel pages, top bar and dropdown windows (winit on X11).
- `src/linux_pop.rs`: dropdown drawing and layout. `src/media_linux.rs`: MPRIS. `src/sys_linux.rs`: system calls and tool wrappers.
- Windows-only modules are left out of Linux builds in `src/main.rs`. Keep logic that both need (no `windows` types) in the shared modules.
- Same rule as Windows: no polling loops where an event exists. Anything that has to poll says so in a `ponytail:` comment.

## Where things are

- `src/ui.rs`: drawing of the pill and every panel page (tab row, cards, Controls, Today, Stats, Clipboard, Apps, Games, Power).
- `src/app.rs`: app state, input, timers and all the actions the pages trigger.
- `src/bar.rs`, `src/pop.rs`: the top bar and its dropdowns.
- `src/buddy.rs`, `src/cast.rs`, `src/scene.rs`, `src/pixdata.rs`: the buddy, the characters and the scripted scenes.
- `src/games.rs`, `src/game.rs`, `src/skins.rs`: the arcade.
- `src/sys.rs`, `src/store.rs`: Windows calls and files under `%APPDATA%\Nock`.
- `build.rs`: embeds the exe icon and file details.

## Seeing your UI changes

There is an ignored test that draws the real panel (and the games) to image files without opening a window, handy when you cannot run the app:

```
set PREVIEW_DIR=C:\some\folder
cargo test preview::render_pages -- --ignored
```

It writes raw BGRA files named `panel_*.raw` (a 4-byte width, a 4-byte height, then the pixels).

## Guidelines

- **Stay light.** No polling loops, no new long-lived background threads, no heavy dependencies. Idle CPU should stay at zero. Anything animated should only tick while it is visible.
- Keep it native: Win32 / Direct2D / WinRT only, no webview.
- Window messages (`WM_APP + n`) must stay unique; the test above will tell you if you reuse one.
- Original art only: please do not add copyrighted images, sounds or character designs to the repository.
- When reporting a bug, include your Windows version and, if present, the logs in `%APPDATA%\Nock` (`crash.log`, `brightness.log`, `scene.log`).
- Small, focused pull requests are easiest to review.
