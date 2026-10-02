use std::error::Error;
use std::path::{Path, PathBuf};

use carronade::error::Error as CarronadeError;
use carronade::history;

fn history(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(name)
        .join("history.toml")
}

#[test]
fn uses_load_back_as_saved() -> Result<(), Box<dyn Error>> {
    let path = history("history-round-trip");
    history::save(&path, history::used(&[], "first", 100))?;
    let uses = history::used(&history::load(&path, 200)?, "second", 200);
    history::save(&path, uses.clone())?;
    assert_eq!(history::load(&path, 300)?, uses);
    Ok(())
}

#[test]
fn a_history_of_0_3_0_loads_one_use_each_a_second_apart() -> Result<(), Box<dyn Error>> {
    let path = history("history-order");
    std::fs::create_dir_all(path.parent().ok_or("no parent")?)?;
    std::fs::write(&path, "launched = [\"b\", \"a\"]\n")?;
    let expected = history::used(&history::used(&[], "a", 999), "b", 1000);
    assert_eq!(history::load(&path, 1000)?, expected);
    Ok(())
}

#[test]
fn a_missing_history_loads_empty() -> Result<(), Box<dyn Error>> {
    assert_eq!(history::load(&history("history-missing"), 0)?, []);
    Ok(())
}

#[test]
fn an_invalid_history_names_its_path() -> Result<(), Box<dyn Error>> {
    let path = history("history-invalid");
    std::fs::create_dir_all(path.parent().ok_or("no parent")?)?;
    std::fs::write(&path, "launched = \"not a list\"\n")?;
    let result = history::load(&path, 0);
    assert!(
        matches!(&result, Err(CarronadeError::StoreParse { path: failed, .. }) if *failed == path),
        "got {result:?}"
    );
    Ok(())
}
