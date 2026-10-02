# carronade

A keyboard launcher for Windows. A hotkey starts it, it shows a popup, and it exits after you pick or cancel. No tray
icon, no background process.

## Install

```powershell
cargo install --path .
```

Copy [config.toml](config.toml) to `%APPDATA%\carronade\config.toml`. carronade does not start without it.

## Usage

```powershell
# launch a Start menu app
carronade drun
# open a file or folder below files.roots
carronade files
# pick a line from stdin and print it
'one', 'two' | carronade dmenu
# use another config
carronade --config C:\path\to\config.toml drun
```

Tab switches between drun and files and keeps the query. Bind `carronade drun` to a hotkey in your window manager.
In GlazeWM, also add an `ignore` window rule for `window_process: { equals: 'carronade' }`.

Matches contain every typed word in any order, ignoring case. Name matches rank above folder matches, and word starts
above matches inside a word. Ties keep list order: recent apps first in drun, shallowest first in files. Enter with no
match runs the typed text as the Run dialog would.

| Key | Action |
| --- | --- |
| Enter | pick the selection, or the typed text when nothing matches |
| Shift+Enter | pick the typed text |
| Ctrl+Enter | in files, start `files.terminal` in the selected folder or the selected file's folder |
| Tab | switch between drun and files |
| Up, Down, Ctrl+P, Ctrl+N | move the selection |
| Left, Right | move the selection a column when the caret is at that end of the query, else move the caret |
| Esc | cancel |

Moving the mouse over a match selects it, the wheel moves the selection a row per notch, and a click picks it.

Exit codes: 0 picked, 1 cancelled, 2 error. Errors go to stderr, or to a message box when there is none.

## Config

Every field is documented in [config.toml](config.toml). All are required except `window.image`, and unknown fields
are errors.

Caches and launch history live in `%LOCALAPPDATA%\carronade\`. They are safe to delete.

## Test

```powershell
cargo clippy --all-targets -- -D warnings
cargo test
```

The integration tests open real windows, so leave the desktop alone while they run.

## License

Apache-2.0, see [LICENSE](LICENSE).
