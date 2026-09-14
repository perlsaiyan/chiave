//! The low-level operations every backend provides, shared by the in-process path
//! and the detached helper process.

use zeroize::Zeroizing;

use crate::clear::ReadClear;
use crate::offers::MimeOffer;
use crate::ClipError;

/// Handle to a copy that is being served.
///
/// On Wayland the offer is served by a thread inside *this* process, so something
/// has to hold it open; `wait()` blocks until another application takes the
/// selection. The command backends (`xclip`, `wl-copy`) fork their own daemon, so
/// their handle is inert.
pub(crate) enum Serving {
    /// Nothing to wait for: an external process owns the selection.
    Detached,
    /// A serving thread in this process.
    Thread(std::thread::JoinHandle<()>),
}

impl Serving {
    /// Block until the selection is taken over by another application. Returns
    /// immediately for [`Serving::Detached`].
    pub(crate) fn wait(self) {
        match self {
            Serving::Detached => {}
            Serving::Thread(handle) => {
                let _ = handle.join();
            }
        }
    }
}

pub(crate) trait RawBackend: ReadClear + Send + Sync {
    /// Take ownership of the clipboard, offering `content` under `offers`.
    ///
    /// Must return as soon as the clipboard has been taken (or an error is known),
    /// never after the content stops being served.
    fn copy(
        &self,
        offers: &[MimeOffer],
        content: &[u8],
        paste_once: bool,
    ) -> Result<Serving, ClipError>;

    /// Name reported by `Clipboard::name()`.
    fn display_name(&self) -> &'static str;

    /// Whether this backend can offer `x-kde-passwordManagerHint`.
    fn supports_sensitive_hint(&self) -> bool {
        true
    }
}

/// Read a pipe/stdout into a zeroizing buffer, mapping empty to `None`.
pub(crate) fn non_empty(bytes: Vec<u8>) -> Option<Zeroizing<Vec<u8>>> {
    if bytes.is_empty() {
        None
    } else {
        Some(Zeroizing::new(bytes))
    }
}
