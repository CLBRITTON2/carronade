//! Drives the built carronade.exe: pipes items in, types through window messages, and checks what it prints.

use std::error::Error;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_RETURN, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CHAR, WM_KEYDOWN,
};
use windows::core::w;

type Outcome = Result<(), Box<dyn Error>>;

// Two pickers on screen take focus from each other, and losing focus cancels one.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

const TIMEOUT: Duration = Duration::from_secs(10);
const CONFIG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml");

fn carronade(config: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_carronade"));
    command.args(["--config", config]);
    command
}

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

struct Picker {
    child: Child,
    window: HWND,
}

struct Exit {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Picker {
    fn open(items: &str) -> Result<Self, Box<dyn Error>> {
        let mut child = carronade(CONFIG)
            .arg("dmenu")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Dropping stdin closes it, which ends the item list.
        child
            .stdin
            .take()
            .ok_or("carronade has no stdin")?
            .write_all(items.as_bytes())?;
        let window = picker_window(child.id())?;
        Ok(Self { child, window })
    }

    /// Posts each UTF-16 unit as WM_CHAR. Posted, not sent, so it stays in order with `press`, as real typing does.
    fn type_query(&self, text: &str) -> Outcome {
        for unit in text.encode_utf16() {
            self.post(WM_CHAR, usize::from(unit))?;
        }
        Ok(())
    }

    fn press(&self, key: VIRTUAL_KEY) -> Outcome {
        self.post(WM_KEYDOWN, usize::from(key.0))
    }

    fn post(&self, message: u32, wparam: usize) -> Outcome {
        unsafe { PostMessageW(Some(self.window), message, WPARAM(wparam), LPARAM(0)) }?;
        Ok(())
    }

    fn exit(mut self) -> Result<Exit, Box<dyn Error>> {
        let start = Instant::now();
        while self.child.try_wait()?.is_none() {
            if start.elapsed() > TIMEOUT {
                self.child.kill()?;
                return Err("carronade did not exit".into());
            }
            sleep(Duration::from_millis(20));
        }
        let output = self.child.wait_with_output()?;
        Ok(Exit {
            code: output.status.code(),
            stdout: String::from_utf8(output.stdout)?,
            stderr: String::from_utf8(output.stderr)?,
        })
    }
}

/// The visible carronade window that `pid` owns, polled for until it shows.
fn picker_window(pid: u32) -> Result<HWND, Box<dyn Error>> {
    let start = Instant::now();
    while start.elapsed() < TIMEOUT {
        let mut after = None;
        while let Ok(window) = unsafe { FindWindowExW(None, after, w!("carronade"), None) } {
            let mut owner = 0;
            unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
            if owner == pid && unsafe { IsWindowVisible(window) }.as_bool() {
                return Ok(window);
            }
            after = Some(window);
        }
        sleep(Duration::from_millis(20));
    }
    Err(format!("no carronade window for process {pid}").into())
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
    let picker = Picker::open("alpha\nbeta\ngamma\n")?;
    picker.type_query("gam")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "gamma");
    Ok(())
}

#[test]
fn enter_with_no_query_prints_the_first_item() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = Picker::open("alpha\nbeta\n")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "alpha");
    Ok(())
}

#[test]
fn down_moves_through_the_matches() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = Picker::open("alpha\nbeta\ngamma\ndelta\n")?;
    picker.type_query("ta")?;
    picker.press(VK_DOWN)?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "delta");
    Ok(())
}

#[test]
fn up_from_the_top_wraps_to_the_last_item() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = Picker::open("alpha\nbeta\ngamma\n")?;
    picker.press(VK_UP)?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "gamma");
    Ok(())
}

#[test]
fn typing_resets_the_cursor_to_the_first_match() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = Picker::open("alpha\nbeta\ngamma\n")?;
    picker.press(VK_DOWN)?;
    picker.type_query("a")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "alpha");
    Ok(())
}

#[test]
fn words_match_in_any_order() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = Picker::open("windows terminal\nterminal preview\n")?;
    picker.type_query("term win")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "windows terminal");
    Ok(())
}

#[test]
fn enter_without_a_match_prints_the_query() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = Picker::open("alpha\nbeta\n")?;
    picker.type_query("zeta")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "zeta");
    Ok(())
}

#[test]
fn unicode_survives_the_round_trip() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = Picker::open("plain\ncafé ☕\n")?;
    picker.type_query("CAFÉ")?;
    picker.press(VK_RETURN)?;
    picked(&picker.exit()?, "café ☕");
    Ok(())
}

#[test]
fn escape_cancels_with_exit_code_1() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = Picker::open("alpha\n")?;
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
    let first = Picker::open("alpha\n")?;
    let second = Picker::open("beta\n")?;
    let exit = first.exit()?;
    assert_eq!((exit.code, exit.stdout.as_str()), (Some(1), ""));
    second.press(VK_ESCAPE)?;
    second.exit()?;
    Ok(())
}

#[test]
fn enter_on_empty_input_cancels() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = Picker::open("")?;
    picker.press(VK_RETURN)?;
    assert_eq!(picker.exit()?.code, Some(1));
    Ok(())
}

#[test]
fn the_caret_moves_through_the_query() -> Outcome {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let picker = Picker::open("")?;
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
    let picker = Picker::open("zeta\n")?;
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
            "carronade: usage: carronade [--config <path>] <dmenu|drun>, got {:?}\n",
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
        text.replace(
            "family = \"Segoe UI Variable Text\"",
            "family = \"No Such Font\"",
        )
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
