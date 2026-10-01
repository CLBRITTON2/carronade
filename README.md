# carronade

A small launcher for Windows. It opens one popup with an input bar over a grid of matches, takes the keyboard, and
closes when it loses focus. It has no tray icon, no background process, and no hotkey of its own: a window manager or
any hotkey tool starts it, and it exits once you pick something or cancel.

## Install

carronade builds with a Rust toolchain (edition 2024). From a checkout:

```powershell
cargo install --path .
```

That puts `carronade.exe` in `%USERPROFILE%\.cargo\bin`. Then copy [config.toml](config.toml) to
`%APPDATA%\carronade\config.toml`. carronade refuses to start without one.

## Modes

```powershell
# launch a Start menu app, recently launched ones first
carronade drun
# search every file and folder below files.roots, and open the pick
carronade files
# pick a line from stdin and print it
'one', 'two', 'three' | carronade dmenu
# read the config from another file
carronade --config C:\path\to\config.toml drun
```

- `drun` lists every app in the Start menu's All apps list, packaged apps included, with their icons. Apps you
  launched through carronade come first, most recent first, and the rest follow by name.
- `files` lists every file and folder below the folders in `files.roots`, shallowest first. It leaves out hidden
  entries and, inside a git repository, whatever its `.gitignore` files ignore.
- `dmenu` lists the lines of stdin and prints the one picked to stdout, for scripts.

drun and files share one window. Tab, or a click on the icon at the input bar's right end, switches between them and
keeps what you typed. So `carronade drun` alone covers both, and `carronade files` only opens on the files first.

## Searching

Type words in any order: a match must contain every word, ignoring case, anywhere in its label. In files the label is
the path below its root, so `integ run` finds `project\tests\integration\run.ps1`.

When nothing matches, Enter picks the typed text itself. drun and files then open it as the Run dialog would: a
program on PATH, a path, or a URL. dmenu prints it.

| Key | Action |
| --- | --- |
| Enter, click | pick the selected match, or the typed text when nothing matches |
| Shift+Enter | pick the typed text |
| Tab, click the icon at the bar's right end | switch between drun and files, keeping the typed text |
| Up, Down, Ctrl+P, Ctrl+N | move the selection |
| Left, Right, Home, End | move the caret |
| Backspace, Delete, Ctrl+Backspace | delete a character or a word |
| Ctrl+V | paste |
| Esc, or click outside | cancel |

Exit codes: 0 for a pick, 1 for a cancel, 2 for an error.

## Binding a hotkey

Bind the command in whatever starts programs on a key. In GlazeWM, an ignore rule also keeps it from managing the
popup:

```yaml
window_rules:
  - commands: ['ignore']
    match:
      - window_process: { equals: 'carronade' }

keybindings:
  - commands: ['shell-exec %USERPROFILE%/.cargo/bin/carronade.exe drun']
    bindings: ['lwin+space']
```

## Config

[config.toml](config.toml) is a complete config, with every field explained in its comments. Every field is required
except `window.image`, and an unknown field is an error, so a config never silently means something other than it
says. It covers:

- `[font]`, `[window]`, `[input]`, `[list]` and `[element]`: the look. Lengths are px, scaled with the monitor's DPI,
  or em, a multiple of the font size. Colors take an optional alpha.
- `input.apps_icon` and `input.files_icon`: the switch icon's text. Any glyph works, such as one from a Nerd Font
  named in `input.prompt_font`.
- `[drun]` and `[files]`: each mode's options, such as `files.roots`.

## Data files

carronade keeps these in `%LOCALAPPDATA%\carronade\`. Deleting any of them is safe.

| File | Holds |
| --- | --- |
| `history.toml` | the apps launched from drun, most recent first |
| `apps.toml` | the apps drun found last time, when `drun.cache` is on |
| `files.toml` | the entries files found last time, when `files.cache` is on |

With a cache on, the picker opens on the list found last time, then lists again after it closes, so it opens fast and
something new shows up from the next run on. `files.toml` takes about 200 bytes per entry, so a root holding 10,000
files and folders makes a file of about 2 MB.

## Errors

carronade writes errors to stderr when it has one, as from a terminal. Started from a hotkey it has none, so it shows
them in a message box titled carronade instead. The commonest is a config that no longer parses, for example after an
upgrade adds a required field: the message names the file and the field, and [config.toml](config.toml) shows what to
add.

## Test

```powershell
cargo clippy --all-targets -- -D warnings
cargo test
```

The integration tests open real picker windows, so leave the desktop alone while they run.

## License

Apache-2.0, see [LICENSE](LICENSE).
