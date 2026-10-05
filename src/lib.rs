//! A launcher and picker for Windows.
//!
//! The lists come from `apps` (the Start menu), `files` (below the configured roots) and `system` (lock, restart and
//! the rest). `picker` shows any list of `picker::Row`s in its window, with `menu` holding the window-free logic it
//! steers by (query parsing, ranking, cursor moves) and `layout` the geometry `config` sets. `shell` opens what was
//! picked. What persists between runs goes through `store` under `%LOCALAPPDATA%\carronade\`: the list caches,
//! `history` (the uses that rank items) and `icons` (the shell icons the picker shows).

pub mod apps;
pub mod config;
pub mod error;
pub mod files;
pub mod history;
pub mod icons;
pub(crate) mod layout;
pub mod menu;
pub mod picker;
pub(crate) mod platform;
pub mod shell;
pub mod store;
pub mod system;
