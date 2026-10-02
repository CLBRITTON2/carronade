//! Lists real folders and drives `carronade files` over them.

mod common;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::PoisonError;

use carronade::error::Error as CarronadeError;
use carronade::{apps, files};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, SetFileAttributesW};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_RETURN, VK_TAB};
use windows::core::HSTRING;

use common::{CONFIG, ONE_AT_A_TIME, Outcome, Picker, carronade};

/// An empty folder for one test, holding the empty `files` named, with their folders.
fn fixture(name: &str, files: &[&str]) -> Result<PathBuf, Box<dyn Error>> {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    for file in files {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().ok_or("a file has no folder")?)?;
        std::fs::write(path, "")?;
    }
    Ok(root)
}

#[test]
fn entries_list_shallowest_first_without_ignored_or_hidden_ones() -> Outcome {
    let root = fixture(
        "files-list",
        &[
            ".git/HEAD",
            ".gitignore",
            "target/out.bin",
            "zet/tests/integration/main.go",
            "zet/README.md",
            "hidden.txt",
        ],
    )?;
    std::fs::write(root.join(".gitignore"), "target/\n")?;
    unsafe {
        SetFileAttributesW(
            &HSTRING::from(root.join("hidden.txt").as_path()),
            FILE_ATTRIBUTE_HIDDEN,
        )
    }?;
    let entries = files::list(std::slice::from_ref(&root))?;
    let labels: Vec<&str> = entries.iter().map(|entry| entry.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "zet",
            "zet\\README.md",
            "zet\\tests",
            "zet\\tests\\integration",
            "zet\\tests\\integration\\main.go",
        ]
    );
    assert!(
        entries
            .iter()
            .all(|entry| Path::new(&entry.path) == root.join(&entry.label))
    );
    Ok(())
}

#[test]
fn a_saved_list_loads_back_unchanged() -> Outcome {
    let root = fixture("files-cache", &["zet/tests/main.go"])?;
    let entries = files::list(std::slice::from_ref(&root))?;
    let path = root.join("files.toml");
    files::save(&path, &entries)?;
    assert_eq!(files::load(&path)?, Some(entries));
    assert_eq!(files::load(&root.join("missing.toml"))?, None);
    Ok(())
}

#[test]
fn a_missing_root_is_named_in_the_error() -> Outcome {
    let missing = fixture("files-missing", &[])?.join("missing");
    let result = files::list(std::slice::from_ref(&missing));
    assert!(
        matches!(&result, Err(CarronadeError::Walk { root, .. }) if *root == missing),
        "got {result:?}"
    );
    Ok(())
}

#[test]
fn the_folder_of_a_file_is_the_one_holding_it() -> Outcome {
    let root = fixture("files-folder", &["zet/README.md"])?;
    let zet = root.join("zet");
    assert_eq!(files::folder(&zet.join("README.md"))?, zet);
    assert_eq!(files::folder(&zet)?, zet);
    let missing = zet.join("missing.md");
    let result = files::folder(&missing);
    assert!(
        matches!(&result, Err(CarronadeError::Attributes { path, .. }) if *path == missing),
        "got {result:?}"
    );
    Ok(())
}

#[test]
fn a_failed_terminal_names_its_program_and_folder() -> Outcome {
    // An empty exe fails to start with no window, whatever the machine's file associations.
    let root = fixture("files-terminal", &["terminal.exe"])?;
    let program = root.join("terminal.exe");
    let program = program.to_str().ok_or("path is not Unicode")?;
    let result = apps::launch_in(program, &root);
    assert!(
        matches!(&result, Err(CarronadeError::LaunchIn { program: failed, folder, .. })
            if failed == program && *folder == root),
        "got {result:?}"
    );
    Ok(())
}

#[test]
fn words_from_anywhere_in_the_path_pick_a_deep_file() -> Outcome {
    picks_the_deep_file("files-search", "files", |picker| {
        picker.type_query("integ run")?;
        picker.press(VK_RETURN)
    })
}

#[test]
fn tab_in_drun_searches_the_files_for_the_query_typed() -> Outcome {
    picks_the_deep_file("files-switch", "drun", |picker| {
        picker.type_query("integ run")?;
        picker.press(VK_TAB)?;
        picker.press(VK_RETURN)
    })
}

/// Opens `mode` over a folder holding a deep `run.exe`, with no caches, and picks it with `keys`.
fn picks_the_deep_file(name: &str, mode: &str, keys: impl Fn(&Picker) -> Outcome) -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let root = fixture(name, &["zet/tests/integration/run.exe", "zet/run.txt"])?;
    let config = root.with_extension("toml");
    // A checkout with core.autocrlf, as on the CI runner, has CRLF line ends.
    let shipped = std::fs::read_to_string(CONFIG)?.replace("\r\n", "\n");
    let roots = "roots = ['~\\dev']\ncache = true";
    let drun = "[drun]\ncache = true";
    for section in [roots, drun] {
        if !shipped.contains(section) {
            return Err(format!("the shipped config has no {section:?}").into());
        }
    }
    let text = shipped
        .replace(
            roots,
            &format!("roots = ['{}']\ncache = false", root.display()),
        )
        .replace(drun, "[drun]\ncache = false");
    std::fs::write(&config, text)?;
    let mut command = carronade(config.to_str().ok_or("config path is not Unicode")?);
    command.arg(mode);
    let picker = Picker::open(command, "")?;
    keys(&picker)?;
    let exit = picker.exit()?;
    // An empty exe fails to start with no window, whatever the machine's file associations, so the error names the
    // pick. Anything that opens a window takes the foreground from the next test's picker.
    let picked = root.join("zet\\tests\\integration\\run.exe");
    let picked = picked.to_str().ok_or("path is not Unicode")?;
    assert_eq!(
        (exit.code, exit.stdout.as_str()),
        (Some(2), ""),
        "{}",
        exit.stderr
    );
    assert!(
        exit.stderr
            .starts_with(&format!("carronade: launching {picked:?} failed: ")),
        "{}",
        exit.stderr
    );
    Ok(())
}
