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
fn launches_load_back_most_recent_first() -> Result<(), Box<dyn Error>> {
    let path = history("history-round-trip");
    history::save(&path, history::launched(&[], "first"))?;
    let recent = history::load(&path)?;
    history::save(&path, history::launched(&recent, "second"))?;
    assert_eq!(history::load(&path)?, ["second", "first"]);
    Ok(())
}

#[test]
fn a_missing_history_loads_empty() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        history::load(&history("history-missing"))?,
        Vec::<String>::new()
    );
    Ok(())
}

#[test]
fn an_invalid_history_names_its_path() -> Result<(), Box<dyn Error>> {
    let path = history("history-invalid");
    std::fs::create_dir_all(path.parent().ok_or("no parent")?)?;
    std::fs::write(&path, "launched = \"not a list\"\n")?;
    let result = history::load(&path);
    assert!(
        matches!(&result, Err(CarronadeError::StoreParse { path: failed, .. }) if *failed == path),
        "got {result:?}"
    );
    Ok(())
}
