# Changelog

## 0.9.5

- Fixed the notch hiding when a Windows overlay (Start, Search, Task View and similar flyouts, which can cover the whole screen) was in front. These are no longer treated as full-screen apps. `visibility.log` showed this on a real PC: a `Windows.UI.Core.CoreWindow` the size of the monitor.

- Tray: *Close the panel when the pointer leaves* (`auto_close` in config.ini). Turn it off and the open panel stays until you close it. When another window takes focus and closes the panel, `visibility.log` now says which window did it.

## 0.9.4

- The joke scenes now have characters that fit them (developer, manager, coworkers, a bug, cricketers, a fan) instead of reusing the fantasy cast.
- The default buddy for new installs is now the Robot (existing settings are kept).

## 0.9.3

- Four new everyday-joke scenes with the original cast: Friday deploy, You're on mute, Low battery and Cricket six (tray menu → Scenes, and they join the random pick).

## 0.9.2

- Fixed the notch and bar staying hidden on some PCs even with nothing full screen. A maximised window on a taskbar set to auto-hide matches the monitor exactly and was mistaken for full screen; now a window must also have no title bar, and the Windows "busy" hint is only trusted for such a window.
- Tray: new *Hide in full-screen apps* switch (`fs_hide` in config.ini) to turn the auto-hide off entirely, and `visibility.log` records why Nock hid.
- Nock's windows no longer have titles, so window pickers in recorders such as OBS should stop listing them as separate windows.

## 0.9.1

- Tray menu → Check for updates: finds the latest GitHub release and, for the installer version, downloads, installs and restarts Nock for you (the plain exe opens the download page). Only runs when you click it.

## 0.9.0

- A softer, more colourful panel: Stats are inset tiles with a colour per metric, Today, Clipboard (colour by kind of item), Controls and Power (colour by severity) all picked up colour, the Media panel is tinted from the album art, and corners are rounder.
- The idle pill is wider and shows the date next to a bold time.
- Games: Thor in Viking Run and a new caped flyer in Super Flight (formerly Thor Flight) are drawn like the scene characters, with outlines, capes and proper poses; richer icons on the game hub.
- README: added measured idle CPU and memory use.

## 0.8.1

- Apps tab: the app picker now looks for new and changed apps every time you open it (the old list stays visible while it updates), and has a Refresh button, so you no longer need to restart Nock to see new apps or fixed icons.

## 0.8.0

- Installer: `NockSetup-<version>.exe` installs Nock just for you (no admin rights), adds it to the Start menu, can start it with Windows, and uninstalls from Windows Settings. The plain `nock.exe` is still published too.
- Apps tab: add apps without editing a file. Click "+ Add app", search your installed apps, click one to pin it, and use the small x to unpin. Shows a spinner while the list loads, with a "Try again" if it takes too long.
- Nock now has its own icon and file details in Explorer instead of the generic Windows icon.
- Nock adds itself to the Start menu so Windows search finds it ("Show in Start menu and search" in the tray menu turns it off).

## 0.7.1

- Controls: removed the Wi-Fi, Bluetooth, Dark mode and Night light tiles (the top bar and Windows' quick settings cover them). The page is now shortcuts: Screenshot, Display, Sound, Network, Power & battery and All settings.

## 0.7.0

- New look for the open panel: a labelled tab row on top (Media, Controls, Today, Stats, Clipboard, Apps, Games, Power) and rounded cards on every page.
- Controls: tiles for Wi-Fi, Bluetooth, Dark mode, Night light, Screenshot and Settings, with their state shown.
- New Stats page: CPU and memory gauges, battery and uptime.
- New Power page: Lock, Sleep, Restart and Shut down (restart and shut down ask for a second click).
- Today: timer, to-dos and calendar in cards.
- Games: fixed text and the character drawing too high after the layout change; tidier corners.

## 0.6.0

- Arcade on the Game tab: Brick Breaker, Slicer, Viking Run and Thor Flight, each with a saved best score.
- Quick capture: press Ctrl+Alt+T anywhere to add a to-do straight from the keyboard.
- Focus timer linked to to-dos: start a session on a task, daily focus and tasks-done stats, optional automatic 5 minute breaks (tray menu).
- Clipboard: pin items (they stay and are saved between runs) and search the history.
- Look-away reminder for your eyes (20-20-20), set from the tray menu.
- The Game tab no longer uses a personal photo.

## 0.5.2

- Media: the notch no longer shows the opposite of what the browser is doing after you pause or resume from a tab (the play/pause state is re-checked shortly after each change).

## 0.5.1

- Top bar volume slider now follows the current default output device immediately (laptop speakers, Bluetooth, monitor).

## 0.5.0

- Mini game on the notch: a click-to-flap game with your photo as the bird (new tab).
- Pill reactions: squish on click, jiggle on hover, a heartbeat when you copy a link, and confetti when the focus timer ends. The pill glows with the music.
- Top bar: clock styles (digital, analog, binary, hours until Friday), an optional CPU and memory read-out, and a water reminder (tray menu).
- Notch: custom focus-timer length (type any number of minutes).
- The photo bobblehead now only appears in the press conference scene.

## 0.4.0

- New scenes: Harry Potter fan art (Harry vs Voldemort, Owl post, Harry and friends vs Voldemort, Quidditch, Dementor); the tray submenu is now just "Scenes".
- Timer: type any length in minutes with the new pencil chip; when it ends, the Quidditch scene plays if there is room.
- Top bar: optional CPU and memory read-out (tray menu, or `stats=1` in `config.ini`).
- Notch: removed the volume and brightness sliders from the Controls page (they live in the top bar).

## 0.3.1

- Scenes: pick a random one each time, never the same twice in a row, and only ones that fit the free space on the bar.

## 0.3.0

- New buddy: a photo bobblehead (Ravi Kishan) that bobs and tilts along the bar and speaks its own lines.
- New scene: Press conference. Reporters with mics and a camera crowd round the bobblehead while it answers.
- Fight scenes: stay clear of the notch pill at its widest, start right after the app name on the left, and pick whichever side has more room.

## 0.2.1

- Fight scenes: use the stretch left of the notch when the right side is too narrow; say so when there is no room.

## 0.2.0

- Pixel-art characters (Ronin, Courier, Necromancer) that talk, as buddies and in three more scenes.
- Fight scenes: four original characters (Sparkplug, Cinder, Krag, Veil) act out three short scripted scenes on the bar now and then; tray menu to play or disable.
- Buddy: an animated character on the top bar (kitty, blob, bunny, hero, robot) that strolls, dances to music, follows the pointer and reacts to timers and connections.

- Connecting animation on the Wi-Fi / Bluetooth bar icons, spinner rows and success / failure notes.
- Hover tooltips for Wi-Fi and Bluetooth showing what is connected.
- Join password-protected Wi-Fi networks from the dropdown.

- Notch with music player, clipboard history, launcher, timer, to-dos, calendar and system controls.
- macOS-style top bar with Wi-Fi, Bluetooth, sound, brightness, battery and clock dropdowns.
- Hides in full screen, with a top-edge reveal.
- Light/dark theme, tray icon, start with Windows.
