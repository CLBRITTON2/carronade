# carronade

A small launcher for Windows. It opens one popup with an input bar over a grid of matches, takes the keyboard,
and closes when it loses focus. It has no tray icon, no background process, and no hotkey of its own: bind it in a
window manager such as GlazeWM.

```powershell
# launch a Start menu app, recently launched ones first, or run the typed text when nothing matches
carronade drun
# pick a line from stdin and print it
'one', 'two', 'three' | carronade dmenu
# search every file and folder below files.roots by words anywhere in the path, and open the pick
carronade files
# read the look from another file
carronade --config C:\Users\Chris\dev\carronade\config.toml drun
```

Exit codes: 0 for a pick, 1 for a cancel, 2 for an error.

| Key | Action |
| --- | --- |
| Enter, click | pick the selected match, or the typed text when nothing matches |
| Shift+Enter | pick the typed text |
| Tab, click the icon at the bar's right end | switch between drun and files, keeping the typed text |
| Up, Down, Ctrl+P, Ctrl+N | move the selection |
| Left, Right, Home, End | move the caret |
| Backspace, Delete, Ctrl+Backspace | delete a character or a word |
| Ctrl+V | paste |
| Esc | cancel |

## Config

carronade reads `%APPDATA%\carronade\config.toml` and fails when it is missing. [config.toml](config.toml) is a
complete one: start from a copy of it. Every field is required except `window.image`, and
unknown fields are errors.

## Install

```powershell
cargo install --path C:\Users\Chris\dev\carronade
```

## Test

```powershell
cargo clippy --all-targets --manifest-path C:\Users\Chris\dev\carronade\Cargo.toml
cargo test --manifest-path C:\Users\Chris\dev\carronade\Cargo.toml
```

The integration tests open real picker windows, so leave the desktop alone while they run.

## License

Apache-2.0, see [LICENSE](LICENSE).
