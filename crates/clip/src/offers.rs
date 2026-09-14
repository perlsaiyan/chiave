//! Pure construction of the MIME offer list.
//!
//! Kept free of I/O so the offer set can be unit-tested headlessly.

/// MIME type that marks clipboard content as a password.
///
/// `wl-paste` exports `CLIPBOARD_STATE=sensitive` when it sees this type, which is
/// what makes cliphist, KDE Klipper and the omarchy quattro clipboard plugin skip
/// the entry instead of recording it in their history.
pub const SENSITIVE_MIME: &str = "x-kde-passwordManagerHint";

/// The value offered under [`SENSITIVE_MIME`].
pub const SENSITIVE_VALUE: &str = "secret";

/// Plain-text MIME types the clipboard content is offered under, in priority order.
pub const TEXT_MIME_TYPES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];

/// What a single offer serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferPayload {
    /// The copied content itself (the secret or the text).
    Content,
    /// A fixed, non-secret marker string.
    Literal(&'static str),
}

/// One MIME type offered to the compositor, and what it serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MimeOffer {
    pub mime: &'static str,
    pub payload: OfferPayload,
}

/// Build the offer list for a copy.
///
/// `sensitive` adds the `x-kde-passwordManagerHint` offer after the text types.
/// The text types come first so that a consumer asking for "any text" gets the
/// content and never the literal `secret` marker.
pub fn build_offers(sensitive: bool) -> Vec<MimeOffer> {
    let mut offers: Vec<MimeOffer> = TEXT_MIME_TYPES
        .iter()
        .map(|mime| MimeOffer {
            mime,
            payload: OfferPayload::Content,
        })
        .collect();
    if sensitive {
        offers.push(MimeOffer {
            mime: SENSITIVE_MIME,
            payload: OfferPayload::Literal(SENSITIVE_VALUE),
        });
    }
    offers
}

/// The primary MIME type used when only one type can be offered (command backends).
pub fn primary_text_mime() -> &'static str {
    TEXT_MIME_TYPES[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_offers_have_no_hint() {
        let offers = build_offers(false);
        assert_eq!(offers.len(), 5);
        assert!(offers.iter().all(|o| o.payload == OfferPayload::Content));
        assert!(!offers.iter().any(|o| o.mime == SENSITIVE_MIME));
    }

    #[test]
    fn secret_offers_carry_the_hint_last() {
        let offers = build_offers(true);
        assert_eq!(offers.len(), 6);
        let last = offers.last().unwrap();
        assert_eq!(last.mime, SENSITIVE_MIME);
        assert_eq!(last.payload, OfferPayload::Literal("secret"));
    }

    #[test]
    fn exact_mime_list_and_order() {
        let mimes: Vec<&str> = build_offers(true).iter().map(|o| o.mime).collect();
        assert_eq!(
            mimes,
            vec![
                "text/plain;charset=utf-8",
                "text/plain",
                "UTF8_STRING",
                "STRING",
                "TEXT",
                "x-kde-passwordManagerHint",
            ]
        );
    }

    #[test]
    fn mimes_are_unique() {
        let offers = build_offers(true);
        let mut mimes: Vec<&str> = offers.iter().map(|o| o.mime).collect();
        mimes.sort_unstable();
        let before = mimes.len();
        mimes.dedup();
        assert_eq!(before, mimes.len());
    }

    #[test]
    fn primary_mime_is_utf8_plain_text() {
        assert_eq!(primary_text_mime(), "text/plain;charset=utf-8");
    }
}
