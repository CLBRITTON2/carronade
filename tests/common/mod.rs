//! Drives the built carronade.exe: types through window messages and collects what it prints.

use std::error::Error;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CHAR, WM_CLOSE,
    WM_KEYDOWN, WM_MOUSEWHEEL,
};
use windows::core::w;

pub type Outcome = Result<(), Box<dyn Error>>;

// Two pickers on screen take focus from each other, and losing focus cancels one.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Holds the desktop for one test's pickers until dropped. A test that panicked holding it leaves nothing to undo.
pub fn turn() -> MutexGuard<'static, ()> {
    ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How long a picker gets to show its window or exit: far above the ~300 ms an uncached list takes, for a busy runner.
const WINDOW_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the harness checks for the window or the exit, short against a picker's ~100 ms startup.
const POLL_INTERVAL: Duration = Duration::from_millis(20);
pub const CONFIG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/config.toml");

pub fn carronade(config: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_carronade"));
    command.args(["--config", config]);
    command
}

pub struct Picker {
    child: Running,
    window: HWND,
}

/// A carronade process, killed on drop if still running, so a test that fails before `exit` takes its window with it
/// instead of leaving it to steal the focus from every later picker.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let ended = match self.0.try_wait() {
            Ok(Some(_)) => Ok(()),
            Ok(None) => self.0.kill().and_then(|()| self.0.wait()).map(drop),
            Err(error) => Err(error),
        };
        if let Err(error) = ended {
            eprintln!("could not end carronade (process {}): {error}", self.0.id());
        }
    }
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
        let mut child = Running(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?,
        );
        // Dropping stdin closes it, which ends dmenu's item list.
        child
            .0
            .stdin
            .take()
            .ok_or("carronade has no stdin")?
            .write_all(stdin.as_bytes())?;
        let window = picker_window(child.0.id())?;
        Ok(Self { child, window })
    }

    pub fn type_query(&self, text: &str) -> Outcome {
        self.type_units(&text.encode_utf16().collect::<Vec<u16>>())
    }

    /// Posts each UTF-16 unit as `WM_CHAR`. Posted, not sent, so it stays in order with `press`, as real typing does.
    pub fn type_units(&self, units: &[u16]) -> Outcome {
        for &unit in units {
            self.post(WM_CHAR, usize::from(unit))?;
        }
        Ok(())
    }

    pub fn press(&self, key: VIRTUAL_KEY) -> Outcome {
        self.post(WM_KEYDOWN, usize::from(key.0))
    }

    /// Turns the mouse wheel by `delta`, positive away from the person, 120 per notch.
    #[allow(
        dead_code,
        reason = "each test binary compiles its own copy, and only dmenu scrolls"
    )]
    pub fn scroll(&self, delta: i16) -> Outcome {
        self.post(
            WM_MOUSEWHEEL,
            usize::from(u16::from_ne_bytes(delta.to_ne_bytes())) << 16,
        )
    }

    /// Asks the window to close, as Alt+F4 or a window manager does.
    #[allow(
        dead_code,
        reason = "each test binary compiles its own copy, and only dmenu closes"
    )]
    pub fn close(&self) -> Outcome {
        self.post(WM_CLOSE, 0)
    }

    fn post(&self, message: u32, wparam: usize) -> Outcome {
        // SAFETY: posting copies the plain integer arguments, so a closed window only makes the call fail.
        unsafe { PostMessageW(Some(self.window), message, WPARAM(wparam), LPARAM(0)) }?;
        Ok(())
    }

    pub fn exit(mut self) -> Result<Exit, Box<dyn Error>> {
        let child = &mut self.child.0;
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if start.elapsed() > WINDOW_TIMEOUT {
                return Err("carronade did not exit".into());
            }
            sleep(POLL_INTERVAL);
        };
        Ok(Exit {
            code: status.code(),
            stdout: read_all(child.stdout.take().ok_or("carronade has no stdout")?)?,
            stderr: read_all(child.stderr.take().ok_or("carronade has no stderr")?)?,
        })
    }
}

/// What an exited carronade left in `pipe`, which its exit closed, so reading cannot block.
fn read_all(pipe: impl Read) -> Result<String, Box<dyn Error>> {
    let mut pipe = pipe;
    let mut text = String::new();
    pipe.read_to_string(&mut text)?;
    Ok(text)
}

/// The visible carronade window that `pid` owns, polled for until it shows.
fn picker_window(pid: u32) -> Result<HWND, Box<dyn Error>> {
    let start = Instant::now();
    while start.elapsed() < WINDOW_TIMEOUT {
        let mut after = None;
        // SAFETY: the class name is a static wide string, and `after` is a window this search returned or none.
        while let Ok(window) = unsafe { FindWindowExW(None, after, w!("carronade"), None) } {
            let mut owner = 0;
            // SAFETY: `owner` is a writable u32, and a window gone since the search only yields 0.
            unsafe { GetWindowThreadProcessId(window, Some(&raw mut owner)) };
            // SAFETY: takes the window by value and only reads its style.
            if owner == pid && unsafe { IsWindowVisible(window) }.as_bool() {
                return Ok(window);
            }
            after = Some(window);
        }
        sleep(POLL_INTERVAL);
    }
    Err(format!("no carronade window for process {pid}").into())
}
