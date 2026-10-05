//! Drives `carronade dmenu`: pipes items in, types, and checks what it prints.

mod common;

use std::error::Error;
use std::io::Write;
use std::process::Stdio;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_UP,
};

use common::{CONFIG, Exit, Outcome, Picker, carronade, turn};

/// What `carronade dmenu` prints to stderr, with `from` replaced by `to` in the shipped config, when it fails before
/// opening.
fn config_error(name: &str, from: &str, to: &str) -> Result<String, Box<dyn Error>> {
    // A checkout with core.autocrlf, as on the CI runner, has CRLF line ends.
    let shipped = std::fs::read_to_string(CONFIG)?.replace("\r\n", "\n");
    if !shipped.contains(from) {
        return Err(format!("the shipped config has no {from:?}").into());
    }
    let path = format!("{}/{name}.toml", env!("CARGO_TARGET_TMPDIR"));
    std::fs::write(&path, shipped.replace(from, to))?;
    let output = carronade(&path)
        .arg("dmenu")
        .stdin(Stdio::null())
        .output()?;
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    Ok(stderr)
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
    let _turn = turn();
    let picker = dmenu("alpha\nbeta\ngamma\n")?;
    picker.type_query("gam")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "gamma");
    Ok(())
}

#[test]
fn an_orphan_surrogate_types_nothing() -> Outcome {
    let _turn = turn();
    // No items, so Enter prints the query itself.
    let picker = dmenu("")?;
    // A low surrogate alone, then a high one followed by a letter instead of its low half.
    picker.type_units(&[0xdc00, 0xd83d, u16::from(b'g')])?;
    picker.type_query("am")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "gam");
    Ok(())
}

#[test]
fn enter_with_no_query_prints_the_first_item() -> Outcome {
    let _turn = turn();
    let picker = dmenu("alpha\nbeta\n")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "alpha");
    Ok(())
}

#[test]
fn down_moves_through_the_matches() -> Outcome {
    let _turn = turn();
    let picker = dmenu("alpha\nbeta\ngamma\ndelta\n")?;
    picker.type_query("ta")?;
    picker.press(VK_DOWN)?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "delta");
    Ok(())
}

#[test]
fn up_from_the_top_wraps_to_the_last_item() -> Outcome {
    let _turn = turn();
    let picker = dmenu("alpha\nbeta\ngamma\n")?;
    picker.press(VK_UP)?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "gamma");
    Ok(())
}

#[test]
fn left_and_right_at_the_query_ends_move_across_columns() -> Outcome {
    let _turn = turn();
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
fn the_wheel_moves_a_row_per_notch_and_stops_at_the_ends() -> Outcome {
    let _turn = turn();
    let picker = dmenu("alpha\nbeta\ngamma\n")?;
    for delta in [120, 120, -60, -60, -120, -360, 120] {
        picker.scroll(delta)?;
    }
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "beta");
    Ok(())
}

#[test]
fn typing_resets_the_cursor_to_the_first_match() -> Outcome {
    let _turn = turn();
    let picker = dmenu("alpha\nbeta\ngamma\n")?;
    picker.press(VK_DOWN)?;
    picker.type_query("a")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "alpha");
    Ok(())
}

#[test]
fn words_match_in_any_order() -> Outcome {
    let _turn = turn();
    let picker = dmenu("harbor charts\ncharts preview\n")?;
    picker.type_query("chart harb")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "harbor charts");
    Ok(())
}

#[test]
fn enter_without_a_match_prints_the_query() -> Outcome {
    let _turn = turn();
    let picker = dmenu("alpha\nbeta\n")?;
    picker.type_query("zeta")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "zeta");
    Ok(())
}

#[test]
fn unicode_survives_the_round_trip() -> Outcome {
    let _turn = turn();
    let picker = dmenu("plain\ncafé ☕\n")?;
    picker.type_query("CAFÉ")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "café ☕");
    Ok(())
}

#[test]
fn escape_cancels_with_exit_code_1() -> Outcome {
    let _turn = turn();
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
fn closing_the_window_cancels() -> Outcome {
    let _turn = turn();
    let picker = dmenu("alpha\n")?;
    picker.close()?;
    let exit = picker.exit()?;
    assert_eq!(
        (exit.code, exit.stdout.as_str(), exit.stderr.as_str()),
        (Some(1), "", "")
    );
    Ok(())
}

#[test]
fn losing_focus_cancels() -> Outcome {
    let _turn = turn();
    let first = dmenu("alpha\n")?;
    let second = dmenu("beta\n")?;
    let exit = first.exit()?;
    assert_eq!(
        (exit.code, exit.stdout.as_str(), exit.stderr.as_str()),
        (Some(1), "", "")
    );
    second.press(VK_ESCAPE)?;
    second.exit()?;
    Ok(())
}

#[test]
fn enter_on_empty_input_cancels() -> Outcome {
    let _turn = turn();
    let picker = dmenu("")?;
    picker.press(VK_RETURN)?;
    let exit = picker.exit()?;
    assert_eq!(
        (exit.code, exit.stdout.as_str(), exit.stderr.as_str()),
        (Some(1), "", "")
    );
    Ok(())
}

#[test]
fn the_caret_moves_through_the_query() -> Outcome {
    let _turn = turn();
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
    let _turn = turn();
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
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert_eq!(
        stderr,
        "carronade: \"show\" is not a mode, use apps, files or dmenu\n\nRun carronade --help for usage.\n"
    );
    Ok(())
}

#[test]
fn no_arguments_print_the_usage_and_fail() -> Outcome {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_carronade")).output()?;
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.starts_with(
            "carronade: a mode is required\n\nUsage: carronade [--config <path>] <mode>\n"
        ),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn help_prints_the_version_and_usage() -> Outcome {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_carronade"))
        .arg("--help")
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(output.status.code(), Some(0), "{stdout}");
    let version = concat!("carronade ", env!("CARGO_PKG_VERSION"), "\n");
    assert!(stdout.starts_with(version), "{stdout}");
    assert!(
        stdout.contains("\nUsage: carronade [--config <path>] <mode>\n"),
        "{stdout}"
    );
    Ok(())
}

#[test]
fn a_missing_config_is_named_in_the_error() -> Outcome {
    let path = format!("{}/missing.toml", env!("CARGO_TARGET_TMPDIR"));
    let output = carronade(&path).arg("dmenu").output()?;
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.starts_with(&format!("carronade: reading the config {path:?} failed: ")),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn items_that_are_not_utf8_are_an_error() -> Outcome {
    let mut child = carronade(CONFIG)
        .arg("dmenu")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Dropping stdin closes it, which ends dmenu's item list.
    child
        .stdin
        .take()
        .ok_or("carronade has no stdin")?
        .write_all(b"alpha\n\xff\n")?;
    let output = child.wait_with_output()?;
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.starts_with("carronade: reading items from stdin failed: "),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn an_unknown_config_field_is_an_error() -> Outcome {
    let stderr = config_error("unknown_field", "[list]", "[list]\ncycle = true")?;
    assert!(
        stderr.contains("is invalid") && stderr.contains("unknown field `cycle`"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn a_missing_font_is_an_error() -> Outcome {
    let stderr = config_error(
        "missing_font",
        "family = \"Segoe UI\"",
        "family = \"No Such Font\"",
    )?;
    assert_eq!(
        stderr,
        "carronade: the font family \"No Such Font\" is not installed\n"
    );
    Ok(())
}

#[test]
fn a_zero_font_size_is_an_error() -> Outcome {
    let stderr = config_error("zero_font_size", "size = 11", "size = 0")?;
    assert!(
        stderr.contains("is invalid")
            && stderr.contains("0 is not a font size from 4 to 72 points"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn a_zero_column_count_is_an_error() -> Outcome {
    let stderr = config_error("zero_columns", "columns = 1", "columns = 0")?;
    assert!(
        stderr.contains("is invalid") && stderr.contains("0 is not a count from 1 to 64"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn an_oversized_icon_is_an_error() -> Outcome {
    let stderr = config_error("oversized_icon", "icon = \"2em\"", "icon = \"5em\"")?;
    assert!(
        stderr.contains("is invalid")
            && stderr.contains("\"5em\" is not an icon side from 1px to 256px or 0.25em to 4em"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn a_relative_root_is_an_error() -> Outcome {
    let stderr = config_error("relative_root", "roots = ['~\\dev']", "roots = ['dev']")?;
    assert!(
        stderr.contains("sets files.roots to \"dev\", which is neither absolute nor below ~"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn a_window_too_large_to_allocate_is_an_error() -> Outcome {
    let _turn = turn();
    let stderr = config_error("huge_window", "width = \"40em\"", "width = \"100000em\"")?;
    assert!(
        stderr.contains("px picker failed, the config's lengths are too large"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn a_missing_image_is_an_error() -> Outcome {
    let image = format!("{}\\missing.png", env!("CARGO_TARGET_TMPDIR"));
    let stderr = config_error(
        "missing_image",
        "# image = ",
        &format!("image = '{image}'\n# "),
    )?;
    assert!(
        stderr.starts_with(&format!("carronade: loading the image {image:?} failed: ")),
        "{stderr}"
    );
    Ok(())
}
