//! Shell icons fetched on a thread of their own, since each takes tens of ms and keys typed meanwhile must not wait.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use super::wic;
use crate::error::Error;
use crate::icons::{self, Pixels};
use crate::platform;

/// Posted to the picker's window after each fetch, so it takes in what `Fetcher::received` holds.
pub(super) const WM_FETCHED: u32 = WM_APP + 1;

/// What the shell gave for one target.
pub(super) enum Fetched {
    Icon {
        target: String,
        pixels: Pixels,
    },
    /// The shell has no item for the target, as for a cached entry deleted since it was listed.
    Gone(String),
}

/// The thread fetching icons. Once this drops, it stops after the fetch under way, whose result has nowhere to go.
pub(super) struct Fetcher {
    requests: Sender<String>,
    results: Receiver<Result<Fetched, Error>>,
}

impl Fetcher {
    /// Starts a thread fetching `side` px icons, which posts `WM_FETCHED` to `window` after each.
    pub(super) fn start(window: HWND, side: u32) -> Result<Self, Error> {
        let (requests, requested) = mpsc::channel();
        let (finished, results) = mpsc::channel();
        let (ready, started) = mpsc::channel();
        let window = Window(window);
        thread::Builder::new()
            .name("icons".to_owned())
            .spawn(move || serve(&window, side, &ready, &requested, &finished))
            .map_err(Error::IconThread)?;
        started.recv().map_err(|_| Error::IconThreadEnded)??;
        Ok(Self { requests, results })
    }

    /// Queues `target`, whose icon comes back through `received`.
    pub(super) fn request(&self, target: String) -> Result<(), Error> {
        self.requests
            .send(target)
            .map_err(|unsent| Error::IconRequest { target: unsent.0 })
    }

    /// Every fetch finished since the last call.
    pub(super) fn received(&self) -> Vec<Result<Fetched, Error>> {
        self.results.try_iter().collect()
    }
}

/// The picker's window, for posting to from the fetcher's thread.
struct Window(HWND);

// SAFETY: a window handle names a window rather than pointing at memory, and PostMessageW takes it on any thread.
unsafe impl Send for Window {}

/// Sends `ready` whether COM and WIC started, then fetches each requested icon into `finished` until the `Fetcher`
/// drops.
fn serve(
    window: &Window,
    side: u32,
    ready: &Sender<Result<(), Error>>,
    requested: &Receiver<String>,
    finished: &Sender<Result<Fetched, Error>>,
) {
    let wic = match platform::com().and_then(|()| wic()) {
        Ok(wic) => wic,
        Err(error) => {
            // `start` waits for this, so the send fails only if it is gone too.
            _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }
    for target in requested {
        let fetched = match icons::fetch(&wic, &target, side) {
            Ok(pixels) => Ok(Fetched::Icon { target, pixels }),
            Err(Error::NoItem { .. }) => Ok(Fetched::Gone(target)),
            Err(error) => Err(error),
        };
        if finished.send(fetched).is_err() {
            return;
        }
        // SAFETY: WM_FETCHED carries no pointers. A destroyed window fails the post, and then nothing waits for more.
        if unsafe { PostMessageW(Some(window.0), WM_FETCHED, WPARAM(0), LPARAM(0)) }.is_err() {
            return;
        }
    }
}
