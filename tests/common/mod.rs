//! Drives the built carronade.exe: types through window messages and collects what it prints.

use std::error::Error;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CHAR, WM_KEYDOWN,
};
use windows::core::w;

pub type Outcome = Result<(), Box<dyn Error>>;

// Two pickers on screen take focus from each other, and losing focus cancels one.
pub static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

const TIMEOUT: Duration = Duration::from_secs(10);
pub const CONFIG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml");

pub fn carronade(config: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_carronade"));
    command.args(["--config", config]);
    command
}

pub struct Picker {
    child: Child,
    window: HWND,
}

pub struct Exit {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Picker {
    /// Starts `command` with `stdin` as its input and waits for its window.
    pub fn open(command: Command, stdin: &str) -> Result<Self, Box<dyn Error>> {
        let mut command = command;
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Dropping stdin closes it, which ends dmenu's item list.
        child
            .stdin
            .take()
            .ok_or("carronade has no stdin")?
            .write_all(stdin.as_bytes())?;
        let window = picker_window(child.id())?;
        Ok(Self { child, window })
    }

    /// Posts each UTF-16 unit as WM_CHAR. Posted, not sent, so it stays in order with `press`, as real typing does.
    pub fn type_query(&self, text: &str) -> Outcome {
        for unit in text.encode_utf16() {
            self.post(WM_CHAR, usize::from(unit))?;
        }
        Ok(())
    }

    pub fn press(&self, key: VIRTUAL_KEY) -> Outcome {
        self.post(WM_KEYDOWN, usize::from(key.0))
    }

    fn post(&self, message: u32, wparam: usize) -> Outcome {
        unsafe { PostMessageW(Some(self.window), message, WPARAM(wparam), LPARAM(0)) }?;
        Ok(())
    }

    pub fn exit(mut self) -> Result<Exit, Box<dyn Error>> {
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
