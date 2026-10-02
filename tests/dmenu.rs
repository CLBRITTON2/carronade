//! Drives `carronade dmenu`: pipes items in, types, and checks what it prints.

mod common;

use std::error::Error;
use std::process::Stdio;
use std::sync::PoisonError;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_UP,
};

use common::{CONFIG, Exit, ONE_AT_A_TIME, Outcome, Picker, carronade};

/// What `carronade dmenu` prints to stderr, with the shipped config edited by `change`, when it fails before opening.
fn config_error(
    name: &str,
    change: impl FnOnce(String) -> String,
) -> Result<String, Box<dyn Error>> {
    let path = format!("{}/{name}.toml", env!("CARGO_TARGET_TMPDIR"));
    std::fs::write(&path, change(std::fs::read_to_string(CONFIG)?))?;
    let output = carronade(&path)
        .arg("dmenu")
        .stdin(Stdio::null())
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    Ok(String::from_utf8(output.stderr)?)
}

fn dmenu(items: &str) -> Result<Picker, Box<dyn Error>> {
    let mut command = carronade(CONFIG);
    command.arg("dmenu");
    Picker::open(command, items)
}

fn picked(exit: &Exit, line: &str) {
    assert_eq!(
        (exit.code, exit.stdout.as_str(), exit.stderr.as_str()),
        (Some(0), format!("{line}\n").as_str(), "")
    );
}

#[test]
fn enter_prints_the_match() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("alpha\nbeta\ngamma\n")?;
    picker.type_query("gam")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "gamma");
    Ok(())
}

#[test]
fn enter_with_no_query_prints_the_first_item() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("alpha\nbeta\n")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "alpha");
    Ok(())
}

#[test]
fn down_moves_through_the_matches() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("alpha\nbeta\ngamma\ndelta\n")?;
    picker.type_query("ta")?;
    picker.press(VK_DOWN)?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "delta");
    Ok(())
}

#[test]
fn up_from_the_top_wraps_to_the_last_item() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("alpha\nbeta\ngamma\n")?;
    picker.press(VK_UP)?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "gamma");
    Ok(())
}

#[test]
fn left_and_right_at_the_query_ends_move_across_columns() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    // The shipped config shows one column of 8, so the next column starts the next page.
    let picker = dmenu("0\n1\n2\n3\n4\n5\n6\n7\n8\n9\n")?;
    for key in [VK_RIGHT, VK_RIGHT, VK_DOWN, VK_LEFT, VK_DOWN] {
        picker.press(key)?;
    }
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "2");
    Ok(())
}

#[test]
fn typing_resets_the_cursor_to_the_first_match() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("alpha\nbeta\ngamma\n")?;
    picker.press(VK_DOWN)?;
    picker.type_query("a")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "alpha");
    Ok(())
}

#[test]
fn words_match_in_any_order() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("windows terminal\nterminal preview\n")?;
    picker.type_query("term win")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "windows terminal");
    Ok(())
}

#[test]
fn enter_without_a_match_prints_the_query() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("alpha\nbeta\n")?;
    picker.type_query("zeta")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "zeta");
    Ok(())
}

#[test]
fn unicode_survives_the_round_trip() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("plain\ncafé ☕\n")?;
    picker.type_query("CAFÉ")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "café ☕");
    Ok(())
}

#[test]
fn escape_cancels_with_exit_code_1() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("alpha\n")?;
    picker.press(VK_ESCAPE)?;
    let exit = picker.exit()?;
    assert_eq!(
        (exit.code, exit.stdout.as_str(), exit.stderr.as_str()),
        (Some(1), "", "")
    );
    Ok(())
}

#[test]
fn losing_focus_cancels() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let first = dmenu("alpha\n")?;
    let second = dmenu("beta\n")?;
    let exit = first.exit()?;
    assert_eq!((exit.code, exit.stdout.as_str()), (Some(1), ""));
    second.press(VK_ESCAPE)?;
    second.exit()?;
    Ok(())
}

#[test]
fn enter_on_empty_input_cancels() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("")?;
    picker.press(VK_RETURN)?;
    assert_eq!(picker.exit()?.code, Some(1));
    Ok(())
}

#[test]
fn the_caret_moves_through_the_query() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("")?;
    picker.type_query("gmma")?;
    for key in [VK_LEFT, VK_LEFT, VK_LEFT] {
        picker.press(key)?;
    }
    picker.type_query("a")?;
    picker.press(VK_END)?;
    picker.type_query("!")?;
    picker.press(VK_HOME)?;
    picker.press(VK_DELETE)?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "amma!");
    Ok(())
}

#[test]
fn backspace_removes_the_character_before_the_caret() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = dmenu("zeta\n")?;
    picker.type_query("zetaa")?;
    picker.press(VK_BACK)?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "zeta");
    Ok(())
}

#[test]
fn unknown_mode_fails_with_usage() -> Outcome {
    let output = carronade(CONFIG).arg("show").output()?;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr)?,
        format!(
            "carronade: usage: carronade [--config <path>] <dmenu|drun|files>, got {:?}\n",
            ["--config", CONFIG, "show"]
        )
    );
    Ok(())
}

#[test]
fn a_missing_config_is_named_in_the_error() -> Outcome {
    let output = carronade("C:/carronade/missing.toml")
        .arg("dmenu")
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.starts_with("carronade: reading the config \"C:/carronade/missing.toml\" failed: "),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn an_unknown_config_field_is_an_error() -> Outcome {
    let stderr = config_error("unknown_field", |text| {
        text.replace("[list]", "[list]\ncycle = true")
    })?;
    assert!(
        stderr.contains("is invalid") && stderr.contains("cycle"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn a_missing_font_is_an_error() -> Outcome {
    let stderr = config_error("missing_font", |text| {
        text.replace("family = \"Segoe UI\"", "family = \"No Such Font\"")
    })?;
    assert_eq!(
        stderr,
        "carronade: the font family \"No Such Font\" is not installed\n"
    );
    Ok(())
}

#[test]
fn a_missing_image_is_an_error() -> Outcome {
    let stderr = config_error("missing_image", |text| {
        text.replace("# image = ", "image = 'C:\\carronade\\missing.png'\n# ")
    })?;
    assert!(
        stderr.starts_with(
            "carronade: loading the image \"C:\\\\carronade\\\\missing.png\" failed: "
        ),
        "{stderr}"
    );
    Ok(())
}
