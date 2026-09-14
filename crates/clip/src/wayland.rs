//! Native Wayland backend built on `wl-clipboard-rs` (ext/wlr-data-control).

use std::io::Read;
use std::sync::mpsc::sync_channel;
use std::thread;

use wl_clipboard_rs::copy::{
    clear as wl_clear, prepare_copy_multi, ClipboardType, MimeSource, MimeType, Options, Seat,
    ServeRequests, Source,
};
use wl_clipboard_rs::paste;
use zeroize::Zeroizing;

use crate::clear::ReadClear;
use crate::offers::{MimeOffer, OfferPayload};
use crate::raw::{non_empty, RawBackend, Serving};
use crate::ClipError;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WaylandRaw;

fn copy_err(err: wl_clipboard_rs::copy::Error) -> ClipError {
    ClipError::Backend(format!("wayland: {err}"))
}

/// True when the failure means "this compositor cannot do data-control", which is
/// the case where falling back to the `wl-copy` binary can still help.
pub(crate) fn is_protocol_missing(err: &ClipError) -> bool {
    match err {
        ClipError::Backend(msg) => {
            msg.contains("is not supported by the compositor") || msg.contains("There are no seats")
        }
        _ => false,
    }
}

fn mime_sources(offers: &[MimeOffer], content: &[u8]) -> Vec<MimeSource> {
    offers
        .iter()
        .map(|offer| {
            let bytes: Box<[u8]> = match offer.payload {
                OfferPayload::Content => content.to_vec().into_boxed_slice(),
                OfferPayload::Literal(value) => value.as_bytes().to_vec().into_boxed_slice(),
            };
            MimeSource {
                source: Source::Bytes(bytes),
                mime_type: MimeType::Specific(offer.mime.to_string()),
            }
        })
        .collect()
}

fn options(paste_once: bool) -> Options {
    let mut opts = Options::new();
    opts.clipboard(ClipboardType::Regular)
        .seat(Seat::All)
        .trim_newline(false)
        // We list every MIME type ourselves, so the crate must not add its own.
        .omit_additional_text_mime_types(true)
        // `prepare_copy_multi` requires foreground mode; we run it on our own
        // thread so that errors from the prepare step are reported synchronously.
        .foreground(true);
    if paste_once {
        opts.serve_requests(ServeRequests::Only(1));
    }
    opts
}

impl RawBackend for WaylandRaw {
    fn copy(
        &self,
        offers: &[MimeOffer],
        content: &[u8],
        paste_once: bool,
    ) -> Result<Serving, ClipError> {
        let opts = options(paste_once);
        let sources = mime_sources(offers, content);

        // `PreparedCopy` is not `Send`, so preparation and serving must happen on
        // the same thread. A rendezvous channel hands the prepare result back so
        // the caller learns about failures before it returns.
        let (tx, rx) = sync_channel::<Result<(), ClipError>>(1);
        let handle = thread::Builder::new()
            .name("chiave-clip-wayland".into())
            .spawn(move || match prepare_copy_multi(opts, sources) {
                Ok(prepared) => {
                    let _ = tx.send(Ok(()));
                    let _ = prepared.serve();
                }
                Err(err) => {
                    let _ = tx.send(Err(copy_err(err)));
                }
            })?;

        match rx.recv() {
            Ok(Ok(())) => Ok(Serving::Thread(handle)),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(ClipError::Backend(
                "wayland: clipboard thread died before taking the selection".into(),
            )),
        }
    }

    fn display_name(&self) -> &'static str {
        "wayland"
    }
}

impl ReadClear for WaylandRaw {
    fn read_current(&self) -> Result<Option<Zeroizing<Vec<u8>>>, ClipError> {
        match paste::get_contents(
            paste::ClipboardType::Regular,
            paste::Seat::Unspecified,
            paste::MimeType::Text,
        ) {
            Ok((mut pipe, _mime)) => {
                let mut buf = Zeroizing::new(Vec::new());
                pipe.read_to_end(&mut buf)?;
                Ok(non_empty(std::mem::take(&mut buf)))
            }
            Err(paste::Error::ClipboardEmpty)
            | Err(paste::Error::NoMimeType)
            | Err(paste::Error::NoSeats) => Ok(None),
            Err(err) => Err(ClipError::Backend(format!("wayland paste: {err}"))),
        }
    }

    fn clear_now(&self) -> Result<(), ClipError> {
        wl_clear(ClipboardType::Regular, Seat::All).map_err(copy_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offers::build_offers;

    #[test]
    fn sources_mirror_the_offer_list() {
        let offers = build_offers(true);
        let sources = mime_sources(&offers, b"hunter2");
        assert_eq!(sources.len(), offers.len());
        for (source, offer) in sources.iter().zip(offers.iter()) {
            assert_eq!(source.mime_type, MimeType::Specific(offer.mime.to_string()));
        }
        let hint = sources.last().unwrap();
        assert_eq!(hint.source, Source::Bytes(b"secret".to_vec().into()));
        assert_eq!(sources[0].source, Source::Bytes(b"hunter2".to_vec().into()));
    }

    #[test]
    fn missing_protocol_is_recognised() {
        assert!(is_protocol_missing(&ClipError::Backend(
            "wayland: A required Wayland protocol (zwlr_data_control_manager_v1 version 1) is not supported by the compositor".into()
        )));
        assert!(!is_protocol_missing(&ClipError::Backend(
            "wayland: Couldn't connect to the Wayland compositor".into()
        )));
    }
}
