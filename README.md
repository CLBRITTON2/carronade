# carronade

A rofi-style launcher for Windows. It opens one popup with a query line and a list, takes the keyboard, and closes
when it loses focus. It has no tray icon, no background process, and no hotkey of its own: bind it in a window manager
such as GlazeWM.

```powershell
# launch a Start menu app, or run the typed text when nothing matches
carronade drun
# pick a line from stdin and print it
'one', 'two', 'three' | carronade dmenu
```

Exit codes: 0 for a pick, 1 for a cancel, 2 for an error.

| Key | Action |
| --- | --- |
| Enter | pick the selected row, or the typed text when nothing matches |
| Shift+Enter | pick the typed text |
| Up, Down, Ctrl+P, Ctrl+N | move the selection |
| Ctrl+Backspace | delete a word |
| Esc | cancel |

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
