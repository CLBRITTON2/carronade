# Release notes

Each release has a `## [<version>] - <date>` section, newest first. `scripts/release.ps1` publishes the section for
the tagged version as its GitHub release notes, and refuses to release a version without one. Changes since the last
release go under `## [Unreleased]`, renamed to the version when it is tagged.

## [Unreleased]

- Icons come from `%LOCALAPPDATA%\carronade\icons.bin` after their first load, which takes well under a millisecond
  instead of tens per icon. A cached icon is loaded again from the shell once it is a week old, and the cache keeps
  the 1000 most recently loaded.
- The Start menu mode is `carronade apps`, and its config section is `[apps]`. Rename `drun` in hotkey bindings and
  `[drun]` in `config.toml`, which otherwise fails to load.
- `carronade dmenu` at the end of a PowerShell pipeline, or with its output assigned, opens the picker and returns
  the line picked. PowerShell used to return at once and leave carronade waiting on its input with no window.
  carronade now needs Windows 11 24H2 or later to start from a hotkey without a console window.
- Matching is fuzzy: a typed word matches when its letters appear in order, so `crnd` finds carronade. Letters in a
  row, at word starts and in the file name rank higher.

## [0.2.0] - 2026-10-02

- Ctrl+Enter in files starts `files.terminal` in the selected folder, or in the folder of the selected file. The new
  `terminal` field in `[files]` is required.
- Left with the caret at the start of the query, or Right at its end, moves the selection to the previous or next
  column.
- Moving the mouse over a match selects it.
- drun lists Lock, Sign out, Hibernate, Restart and Shut down after the apps, with icons. Picking one runs it at
  once.
- The mouse wheel moves the selection a row per notch, stopping at either end of the list.
- Ctrl+Shift+Enter in drun launches the selected app as administrator. Declining the UAC prompt exits as a cancel.
- files lists the entries opened recently first, as drun does with apps. Their history lives in
  `%LOCALAPPDATA%\carronade\files-history.toml`.

## [0.1.0] - 2026-10-01

First release.

- `drun` launches Start menu apps with their icons, recently launched ones first.
- `files` opens a file or folder below `files.roots`, honoring `.gitignore`. Tab, or the icon at the end of the input
  bar, switches between drun and files and keeps the query.
- `dmenu` picks a line from stdin and prints it.
- Matches contain every typed word in any order. Name matches rank above folder matches, and word starts above matches
  inside a word. Enter with no match runs the typed text as the Run dialog would.
- One `config.toml` sets the window, colors, fonts, an optional background image and caching. Paths can start with
  `~`.
