<p align="center"><img src="assets/icon256.png" width="96" alt="Nock"></p>

<h1 align="center">Nock</h1>

<p align="center">A tiny, native Windows <b>dynamic island</b> and <b>macOS-style menu bar</b>.<br>
Written in Rust. No webview, no framework, no runtime. One small <code>.exe</code>.</p>

<p align="center">
<a href="https://github.com/Akhilesh-Varute/Nock/actions/workflows/ci.yml"><img src="https://github.com/Akhilesh-Varute/Nock/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
<img src="https://img.shields.io/badge/platform-Windows%2010%2F11-blue" alt="Windows 10/11">
<img src="https://img.shields.io/badge/license-MIT-green" alt="MIT">
</p>

> **Status: early.** It type-checks and builds in CI, but has had little real-world testing. Expect rough edges and please [open an issue](../../issues).

## Features

- **Notch / dynamic island** at the top centre: now-playing music (any app via Windows media sessions), clipboard history, app and folder launcher, focus timer, to-dos, calendar, volume and brightness.
- **Top bar** like the macOS menu bar: active app name, Wi-Fi, Bluetooth, sound, brightness, battery and clock, each with a dropdown (join Wi-Fi networks, connect Bluetooth devices, sliders, calendar).
- **Zero idle cost**: event driven, no polling loop. Hides itself in full-screen video and games, with a top-edge reveal.
- Follows the Windows light/dark theme.

## Build

1. Install Rust from <https://rustup.rs> (choose the default **MSVC** toolchain). If asked, also install the Visual Studio Build Tools with the "Desktop development with C++" workload.
2. In this folder run:

   ```
   cargo build --release
   ```

   (or double-click `build.bat`)
3. Run `target\release\nock.exe`. It is a single self-contained file. Copy it anywhere.

Right-click the tray icon → **Start with Windows** to launch it at login.

## Top bar

A thin translucent bar runs along the top of the screen (like the macOS menu bar) and the notch hangs from its middle. Left: the app you are in. Right: Wi-Fi, Bluetooth, volume, battery and the date and time. Clicking an icon opens a small dropdown right under it:

- **Wi-Fi**: on/off switch, nearby networks with signal strength (the connected one marked). Click a saved network to join it, click the connected one to disconnect. A new password-protected network asks for its password right in the dropdown (paste works); an open network joins directly. Enterprise networks open Windows' own Wi-Fi settings.
- **Bluetooth**: on/off switch, paired devices (connected ones marked). Click a device to connect or disconnect it. Pairing new devices is done in Windows' Bluetooth settings (linked).
- **Sound**: volume slider and mute.
- **Display**: brightness slider (laptop panels; always shown).
- **Battery**: percentage, charging state, time left, battery saver, link to battery settings.
- **Date and time**: month calendar with today highlighted.

**Connecting feedback:** while a Wi-Fi or Bluetooth connection is in progress the bar icon pulses and the row shows a spinner. When it finishes a small note says "Connected to …" or "Couldn't connect …".

**Hover tooltips:** resting the pointer on the Wi-Fi or Bluetooth icon shows what is connected (network name and signal; connected Bluetooth devices) without clicking.

A dropdown closes when the pointer leaves it, when you click into another window, or when you click its icon again. The bar follows the Windows light/dark setting; the notch itself always stays black. It reserves its space like the taskbar does, so maximised windows start below it. Tray → *Top bar* turns it off (the old notch-only behaviour returns); `top_bar` in `config.ini` does the same.

## Buddy

A tiny original character lives on the top bar, in the free stretch between the notch and the status icons. It naps most of the time and wakes up for short outings, so it costs almost nothing:

- **Strolls** along the bar every minute or so, then goes back to sleep.
- **Dances** when music starts playing.
- **Follows your pointer** when you move it over that part of the bar, and its eyes follow the cursor. Click it to pet it.
- **Cheers** when the focus timer ends, and says "Connected!" or "Oops" after a Wi-Fi / Bluetooth attempt, "Low battery" when the battery runs low, and "Bedtime?" late at night.

Pick the character from the tray menu → **Buddy**: Kitty (default), Blob, Bunny, Hero or Robot, or Off. It is also the `buddy` setting in `config.ini` (0 = off, 1 blob, 2 cat, 3 bunny, 4 hero, 5 robot). The characters are drawn from simple shapes in code, so there are no image files. It hides together with the bar in full screen.

## Using it

| Action | What happens |
| --- | --- |
| Click the pill | Opens the panel |
| Move the mouse away | Panel closes after a short delay |
| Pin icon (top-left of panel) / `Ctrl+Alt+N` | Keeps it open until you collapse it |
| Chevron (top-right) | Collapses it |
| Mouse wheel over the pill | Scroll down opens it |
| Mouse wheel over the bottom tab bar | Switches pages |
| `Ctrl+Alt+H` / tray → *Hide notch* | Tucks the notch away completely; press again (or `Ctrl+Alt+N`, or click the tray icon) to bring it back |
| Tray icon | Start with Windows, reserve screen space, edit/reload pinned apps, open settings folder, quit |

Pages (bottom tab bar):

- **Music**: artwork, title, artist, source app, seekable progress bar, previous / play-pause / next. Reads Windows' own media session, so it works with Spotify, YouTube and other browser media, VLC, Apple Music and so on. While music plays, the collapsed pill shows the artwork, track and equaliser bars.
- **Controls**: volume slider + mute, brightness slider, and screenshot (Snipping Tool), lock and sleep.
- **Clipboard**: the last 20 things you copied. Click one to paste it into the app you were using. History lives in memory only (never written to disk), and copies from password managers are skipped.
- **Apps**: pinned launcher tiles with the real app icons.
- **Tasks**: focus timer (5/15/25/50 min presets; a running timer shows in the pill; when it finishes you get three beeps and the pill pulses "Time's up" for 30 s, or until you click it), a 5-item to-do list (saved to disk), and a month calendar.

## Icon

The tray icon is `assets/icon.png`, compiled into the exe. `assets/notch.ico` is the same artwork as a Windows icon file, handy for a shortcut (right-click the shortcut → Properties → Change Icon).

## Files and settings

Everything is in `%APPDATA%\Nock\` (tray → *Open settings folder*):

- `config.ini`: `reserve_space` (keep a thin strip at the top so maximised windows, e.g. browser tabs, sit below the notch; also in the tray menu), `hover_expand` (open on hover instead of click), `eq_animation` (animate the equaliser bars), `clock_24h`.
- `apps.txt`: pinned launcher entries, one per line: `Name|target|optional arguments`. The target can be an exe, folder, URL or `ms-settings:` link. After editing, use tray → *Reload pinned apps*.
- `todos.txt`: the to-do list.
- `brightness.log`: only written if the brightness query or change fails, with the reason.

## Why it is light

- **Zero work while idle.** The process blocks in `GetMessage()`. There is no polling loop anywhere.
- Timers exist only while needed: a ~60 fps animation timer only during expand/collapse, a 20 fps timer only for the equaliser bars while music is playing and the pill is collapsed, and a 1 Hz timer only while the panel is open on a playing track / the Controls page, or while a focus timer runs. The clock re-arms once per minute.
- Now-playing, clipboard changes, foreground-window changes and hotkeys are all event driven (WinRT events, `WM_CLIPBOARDUPDATE`, `SetWinEventHook`).
- The bar and notch hide while a game or full-screen video/browser is active. During a full-screen video or browser (not games) you can bring them back by resting the pointer on the top edge near the middle of the screen; they slide away again once you move down. While that is active the app watches the pointer position with a tiny system hook that exists only during full screen.
- Drawing is Direct2D into a small bitmap, only the size of the current pill/panel, pushed to a per-pixel-alpha layered window. Artwork is decoded once per track.
- One extra thread (media session). Release profile uses LTO, one codegen unit and `panic=abort`.

## Known limits

- Primary monitor only.
- Wi-Fi and Bluetooth buttons toggle the radio through the Windows Radios API. If Windows exposes no such radio (or denies access) the button opens the matching Settings page instead.
- Brightness on laptop panels is set through the display driver directly (instant, follows the slider as you drag). If the driver does not offer that, it falls back to a slower WMI call through PowerShell.
- *Sleep* uses the standard suspend call; if hibernation is enabled on your PC Windows may hibernate instead.
- The calendar shows the month only (no events).
- The to-do text box takes typed characters only (no IME composition).

## Contributing

Issues and pull requests are welcome, see [CONTRIBUTING.md](CONTRIBUTING.md). Version history is in [CHANGELOG.md](CHANGELOG.md).

## License

[MIT](LICENSE)
