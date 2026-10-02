# carronade

A keyboard launcher for Windows. A hotkey starts it, it shows a popup, and it exits after you pick or cancel. No tray
icon, no background process.

## Install

```powershell
cargo install --path .
```

Copy [config.toml](config.toml) to `%APPDATA%\carronade\config.toml`. carronade does not start without it.

Requires Windows 11 24H2 or later (an older Windows also opens a console window when a hotkey starts carronade).

## Usage

```powershell
# launch a Start menu app
carronade apps
# open a file or folder below files.roots
carronade files
# pick a line from stdin and print it
'one', 'two' | carronade dmenu
# use another config
carronade --config C:\path\to\config.toml apps
```

apps also lists Lock, Sign out, Hibernate, Restart and Shut down after the apps, each with a Segoe MDL2 Assets icon.
Picking one runs it at once, with no confirmation. Tab switches between apps and files and keeps the query. Bind
`carronade apps` to a hotkey in your window manager.
In GlazeWM, also add an `ignore` window rule for `window_process: { equals: 'carronade' }`.

Matches hold the letters of every typed word in order, with gaps allowed, so `crnd` finds carronade. Words match in
any order, ignoring case. Letters in a row and at word starts, including capitals inside a name like GlazeWM, rank
higher, and so do matches in the file name over the folders. Apps and entries you open often or lately rank higher
too, by a bonus that grows with use but never lifts a poor match over a good one, and with nothing typed they come
first. Ties keep list order: by name in apps and shallowest first in files. The letters matched are drawn in
`element.highlight`. Enter with no match runs the typed text as the Run dialog would.

A word can start or end with an operator:

| Word | Matches |
| --- | --- |
| `'word` | the letters in a row anywhere |
| `^word` | items starting with it |
| `word$` | items ending with it, such as `.rs$` |
| `^word$` | the whole item |
| `!word` | items without it in a row, also `!^word` and `!word$` |

| Key | Action |
| --- | --- |
| Enter | pick the selection, or the typed text when nothing matches |
| Shift+Enter | pick the typed text |
| Ctrl+Enter | in files, start `files.terminal` in the selected folder or the selected file's folder |
| Ctrl+Shift+Enter | in apps, launch the selected app as administrator |
| Tab | switch between apps and files |
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
