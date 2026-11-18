//! The Dock icon's cursor blinks while any terminal is working, as the
//! terminal glyph's does in the app (`.icon-blink`: 1.1 s, lit first). The
//! badge beside it is the count waiting (`update_dock` in `lib.rs`).
//!
//! The Dock can't animate an icon itself, so each blink is a swap between
//! two pre-rendered images: `icon.png` and `dock-cursor-off.png`, the same
//! size from the same renderer, so nothing but the cursor moves. That is
//! two Dock redraws a blink, and none while nothing works. When the last
//! terminal stops, the icon the app started with is put back.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

/// Half of the app's 1.1 s `cursor-blink`.
const HALF: Duration = Duration::from_millis(550);

pub struct Blink {
    working: Arc<AtomicBool>,
    thread: Thread,
}

impl Blink {
    /// A thread that sleeps while nothing works, and swaps the icon every
    /// `HALF` while something does.
    pub fn start(app: tauri::AppHandle) -> Self {
        let working = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&working);
        let handle = thread::Builder::new()
            .name("dock-blink".into())
            .spawn(move || {
                let mut lit = true;
                loop {
                    if !flag.load(Ordering::Acquire) {
                        if !lit {
                            lit = true;
                            show(&app, Look::Rest);
                        }
                        thread::park();
                        continue;
                    }
                    // Wait out this half, noticing promptly if work stops.
                    let deadline = Instant::now() + HALF;
                    while flag.load(Ordering::Acquire) {
                        let now = Instant::now();
                        if now >= deadline {
                            break;
                        }
                        thread::park_timeout(deadline - now);
                    }
                    if flag.load(Ordering::Acquire) {
                        lit = !lit;
                        show(&app, if lit { Look::Lit } else { Look::Off });
                    }
                }
            })
            .expect("spawn the dock-blink thread");
        Blink {
            working,
            thread: handle.thread().clone(),
        }
    }

    /// Whether anything is working. Starting begins with the cursor lit, so
    /// a brief `working` never blinks it off.
    pub fn set_working(&self, working: bool) {
        if self.working.swap(working, Ordering::AcqRel) != working {
            self.thread.unpark();
        }
    }
}

#[derive(Clone, Copy)]
enum Look {
    /// The icon the app had before the first blink.
    Rest,
    Lit,
    Off,
}

#[cfg(target_os = "macos")]
fn show(app: &tauri::AppHandle, look: Look) {
    let _ = app.run_on_main_thread(move || macos::show(look));
}

#[cfg(not(target_os = "macos"))]
fn show(_app: &tauri::AppHandle, _look: Look) {}

#[cfg(target_os = "macos")]
mod macos {
    use super::Look;
    use objc2::rc::Retained;
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    use std::cell::OnceCell;

    pub(super) const LIT: &[u8] = include_bytes!("../icons/icon.png");
    pub(super) const OFF: &[u8] = include_bytes!("../icons/dock-cursor-off.png");

    struct Images {
        /// `None` if the app had no icon of its own set, which restores the
        /// bundle's.
        rest: Option<Retained<NSImage>>,
        lit: Retained<NSImage>,
        off: Retained<NSImage>,
    }

    thread_local! {
        /// Made on the main thread at the first blink, before the icon is
        /// first changed, so `rest` is what the app started with.
        static IMAGES: OnceCell<Option<Images>> = const { OnceCell::new() };
    }

    pub(super) fn image(png: &[u8]) -> Option<Retained<NSImage>> {
        NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png))
    }

    pub(super) fn show(look: Look) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        IMAGES.with(|cell| {
            let images = cell.get_or_init(|| {
                Some(Images {
                    rest: app.applicationIconImage(),
                    lit: image(LIT)?,
                    off: image(OFF)?,
                })
            });
            let Some(images) = images else { return };
            let icon = match look {
                Look::Rest => images.rest.as_deref(),
                Look::Lit => Some(&*images.lit),
                Look::Off => Some(&*images.off),
            };
            // SAFETY: on the main thread, with an image or `None`, which
            // AppKit documents as restoring the bundle's icon.
            unsafe { app.setApplicationIconImage(icon) };
        });
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::macos::{image, LIT, OFF};

    #[test]
    fn both_icons_decode_at_the_same_size() {
        let lit = image(LIT).expect("icon.png decodes");
        let off = image(OFF).expect("dock-cursor-off.png decodes");
        let (a, b) = (lit.size(), off.size());
        assert!(a.width > 0.0 && a.height > 0.0);
        assert_eq!((a.width, a.height), (b.width, b.height));
    }
}
