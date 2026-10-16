# Good first issues

Ideas that are well contained and don't need deep knowledge of the code. Pick one, open an issue to say you're on it, and send a pull request.

- **Faster brightness changes.** The slider can feel slow on some laptops (the fallback uses WMI through PowerShell). Look at `src/sys.rs` and see if the direct display-driver path can be used more often, or the change batched / debounced.
- **Add a pixel-art buddy.** Characters are 26x28 grids in `proto/emit.py` style data (`src/pixdata.rs`) and drawn in `src/cast.rs`. Design a new one and a few lines of dialogue.
- **New fight scene.** Scenes are keyframe scripts in `src/scene.rs` (`script()`); the unit tests check they are well formed.
- **Second monitor support.** Nock uses the primary monitor only.
- **Signal strength icon** for Wi-Fi in the bar (instead of the plain icon).
- **Bluetooth battery levels** in the dropdown where devices report them.
- **Forget a saved Wi-Fi network / unpair a Bluetooth device** from the dropdowns.
- **Non-English Wi-Fi parsing.** Network info comes from `netsh` output parsed in English.
