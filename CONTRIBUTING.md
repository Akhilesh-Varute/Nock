# Contributing

Thanks for helping! Nock targets Windows 10/11 and is plain Rust against the `windows` crate.

## Build and check

```
cargo build --release
cargo test
cargo clippy
```

## Guidelines

- **Stay light.** No polling loops, no new background threads unless needed, no heavy dependencies. Idle CPU should stay at zero.
- Keep it native: Win32 / Direct2D / WinRT only, no webview.
- When reporting a bug, include your Windows version and, for brightness problems, `%APPDATA%\Nock\brightness.log`.
- Small, focused pull requests are easiest to review.
