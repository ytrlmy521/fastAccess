# FastAccess

FastAccess is a lightweight Windows launcher that combines event-driven File
Explorer folder navigation with files maintained by Windows Shell in
`FOLDERID_Recent`.

Press **Alt + Shift + Space**, type a few characters, and press **Enter** to
open the selected recent file or folder.

## MVP scope

- Reads `.lnk` shortcuts from Windows Recent Items.
- Records completed File Explorer folder navigation through Shell COM events;
  no polling timer is used.
- Resolves filesystem targets with the `lnk` crate.
- Searches an in-memory snapshot with `nucleo-matcher`.
- Moves items opened through FastAccess to the top immediately and persists
  that access time.
- Refreshes Windows Recent Items whenever the launcher is shown.
- Keeps at most 500 history items and coalesces cache writes on one background
  writer thread.
- Stores a versioned JSON cache in `%LOCALAPPDATA%\FastAccess\cache.json`.
- Uses an atomic cache replacement strategy.
- Runs Recent collection and cache I/O on dedicated bounded worker queues.
- Provides a native Slint UI and a Win32 global hotkey.

It does not implement full-disk search, content search, Jump Lists, browser
history, Office-specific MRU lists, or NTFS/USN indexing.

## Prerequisites

- Windows 10 or Windows 11
- Rust stable with the MSVC toolchain
- Visual Studio Build Tools with **Desktop development with C++**

Install Rust from <https://rustup.rs/>, then select the Windows MSVC target:

```powershell
rustup default stable-x86_64-pc-windows-msvc
```

The pinned Slint 1.17.1 dependency requires Rust 1.92 or newer.

## Run

```powershell
cargo run --release
```

The application opens once at startup. Press `Esc` to hide it. The global
shortcut toggles it afterward. Press `Ctrl+Q` while the window is visible to
exit the background process.

## Test

```powershell
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

## Build a release executable

```powershell
cargo build --release
```

The executable is written to:

```text
target\release\fastaccess.exe
```

Alternatively, run `scripts\build-release.ps1`; it formats, tests, runs Clippy,
builds the release executable, and places a ZIP package in `dist`.

## Source layout

```text
src/
├── main.rs                 Application composition and worker/UI communication
├── lib.rs
├── model/
│   └── recent_item.rs      Serializable in-memory model
├── recent/
│   ├── history.rs          Bounded history merge, ordering, and deduplication
│   ├── known_folder.rs     FOLDERID_Recent resolution
│   ├── scanner.rs          Shortcut enumeration, ordering, and deduplication
│   └── shortcut.rs         .lnk target parsing
├── search/
│   └── matcher.rs          In-memory fuzzy matching and Top-K
├── cache/
│   ├── storage.rs          Versioned JSON cache and atomic replacement
│   └── writer.rs           Bounded, coalescing background cache writer
├── platform/
│   ├── explorer.rs         Event-driven Explorer folder navigation tracking
│   ├── hotkey.rs           Alt+Shift+Space registration
│   └── launcher.rs         Shell target opening
└── ui/
    ├── app.slint           Native UI
    └── icons/              Bundled Lucide SVG icons
```

## Known MVP limitations

- `lnk::ShellLink::link_target()` resolves targets represented by the shortcut's
  `LINK_INFO` structure. Shortcuts without a filesystem target are skipped.
- A shortcut's modification time is used as `observed_at_ms`; it is not claimed
  to be NTFS last-access time.
- File Explorer folder navigation is tracked while FastAccess is running.
  Folder visits that happened before FastAccess started cannot be reconstructed
  exactly.
- Files opened through other applications appear when Windows creates or
  updates a Recent Items shortcut.
- Virtual Shell locations such as Control Panel are ignored because they do not
  have a filesystem path.
- If another application already owns Alt+Shift+Space, startup reports an error.
- The first release must be measured on representative Windows hardware before
  making P95 latency or memory guarantees.

## Technical references

- [Microsoft: `FOLDERID_Recent` and Known Folder IDs](https://learn.microsoft.com/windows/win32/shell/knownfolderid)
- [Microsoft: `SHGetKnownFolderPath`](https://learn.microsoft.com/windows/win32/api/shlobj_core/nf-shlobj_core-shgetknownfolderpath)
- [Microsoft: `RegisterHotKey`](https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-registerhotkey)
- [Slint Rust documentation](https://docs.slint.dev/latest/docs/rust/slint/)
- [`lnk` crate documentation](https://docs.rs/lnk/0.6.4/lnk/)
- [`nucleo-matcher` crate documentation](https://docs.rs/nucleo-matcher/0.3.1/nucleo_matcher/)

## License

MIT

Bundled Lucide icons retain their upstream notices in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
