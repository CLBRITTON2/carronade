//! Reads the real Start menu through the shell.

use std::error::Error;

use carronade::apps;
use carronade::error::Error as CarronadeError;

#[test]
fn list_finds_named_apps_in_name_order() -> Result<(), Box<dyn Error>> {
    let apps = apps::list()?;
    assert!(!apps.is_empty(), "the Start menu listed no apps");
    assert!(
        apps.iter()
            .all(|app| !app.name.is_empty() && !app.id.is_empty())
    );
    let names: Vec<String> = apps.iter().map(|app| app.name.to_lowercase()).collect();
    assert!(names.is_sorted());
    Ok(())
}

#[test]
fn list_includes_packaged_apps() -> Result<(), Box<dyn Error>> {
    // Packaged apps have an AUMID, family name then `!` then app id. Settings ships with every Windows 11 install.
    let apps = apps::list()?;
    assert!(apps.iter().any(|app| app.id.starts_with("windows.immersivecontrolpanel_") && app.id.contains('!')));
    Ok(())
}

#[test]
fn launching_a_missing_app_names_it_in_the_error() {
    let target = "shell:AppsFolder\\carronade.missing_0000000000000!App";
    let result = apps::launch(target);
    assert!(
        matches!(&result, Err(CarronadeError::Launch { target: failed, .. }) if failed == target),
        "got {result:?}"
    );
}
