use std::error::Error;
use std::path::{Path, PathBuf};

use carronade::error::Error as CarronadeError;
use carronade::history;
use carronade::store::UnixSeconds;

fn history(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(name)
        .join("history.toml")
}

#[test]
fn uses_load_back_as_saved() -> Result<(), Box<dyn Error>> {
    let path = history("history-round-trip");
    history::save(&path, history::used(&[], "first", UnixSeconds(100)))?;
    let uses = history::used(
        &history::load(&path, UnixSeconds(200))?,
        "second",
        UnixSeconds(200),
    );
    history::save(&path, uses.clone())?;
    assert_eq!(history::load(&path, UnixSeconds(300))?, uses);
    Ok(())
}

#[test]
fn a_history_of_0_3_0_loads_one_use_each_a_second_apart() -> Result<(), Box<dyn Error>> {
    let path = history("history-order");
    std::fs::create_dir_all(path.parent().ok_or("no parent")?)?;
    std::fs::write(&path, "launched = [\"b\", \"a\"]\n")?;
    let expected = history::used(
        &history::used(&[], "a", UnixSeconds(999)),
        "b",
        UnixSeconds(1000),
    );
    assert_eq!(history::load(&path, UnixSeconds(1000))?, expected);
    Ok(())
}

#[test]
fn a_saved_history_starts_with_its_version() -> Result<(), Box<dyn Error>> {
    let path = history("history-versioned");
    history::save(&path, history::used(&[], "a", UnixSeconds(100)))?;
    assert!(std::fs::read_to_string(&path)?.starts_with("version = 1\n"));
    Ok(())
}

#[test]
fn a_history_without_a_version_loads_as_the_current_one() -> Result<(), Box<dyn Error>> {
    let path = history("history-unversioned");
    std::fs::create_dir_all(path.parent().ok_or("no parent")?)?;
    std::fs::write(&path, "[[used]]\nkey = \"a\"\ncount = 2\nlast = 50\n")?;
    let expected = history::used(
        &history::used(&[], "a", UnixSeconds(40)),
        "a",
        UnixSeconds(50),
    );
    assert_eq!(history::load(&path, UnixSeconds(100))?, expected);
    Ok(())
}

#[test]
fn a_history_of_another_version_names_both() -> Result<(), Box<dyn Error>> {
    let path = history("history-other-version");
    std::fs::create_dir_all(path.parent().ok_or("no parent")?)?;
    std::fs::write(&path, "version = 2\nused = []\n")?;
    let result = history::load(&path, UnixSeconds(0));
    assert!(
        matches!(&result, Err(CarronadeError::StoreVersion { path: failed, found, expected: 1 })
            if *failed == path && found == "2"),
        "got {result:?}"
    );
    Ok(())
}

#[test]
fn a_missing_history_loads_empty() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        history::load(&history("history-missing"), UnixSeconds(0))?,
        []
    );
    Ok(())
}

#[test]
fn an_invalid_history_names_its_path() -> Result<(), Box<dyn Error>> {
    let path = history("history-invalid");
    std::fs::create_dir_all(path.parent().ok_or("no parent")?)?;
    std::fs::write(&path, "launched = \"not a list\"\n")?;
    let result = history::load(&path, UnixSeconds(0));
    assert!(
        matches!(&result, Err(CarronadeError::StoreParse { path: failed, .. }) if *failed == path),
        "got {result:?}"
    );
    Ok(())
}
