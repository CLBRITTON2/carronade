# Release notes

Each release has a `## [<version>] - <date>` section, newest first. `scripts/release.ps1` publishes the section for
the tagged version as its GitHub release notes, and refuses to release a version without one. Changes since the last
release go under `## [Unreleased]`, renamed to the version when it is tagged.

## [Unreleased]

- Ctrl+Enter in files starts `files.terminal` in the selected folder, or in the folder of the selected file. The new
  `terminal` field in `[files]` is required.

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
