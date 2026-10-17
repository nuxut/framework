//! Linux dışı yedek (ADR-0003 kural 2): xdg-desktop-portal yok; her istek
//! [`PortalError::Unavailable`] ile biter. Teslim yine istek thread'inden, tam bir kez
//! (`Portal::start` sözleşmesi değişmez).

use std::path::PathBuf;

use crate::{Inner, OpenFileOptions, PortalError, PortalRequestId, SaveFileOptions};

// Linux dışında seçenekler okunmaz; tür Linux yoluyla aynı kalsın diye alanlar durur.
#[allow(dead_code)]
pub(crate) enum Kind {
    Open(OpenFileOptions),
    Save(SaveFileOptions),
}

/// Linux yoluyla aynı imza; hemen `Unavailable`.
pub(crate) fn run(
    _inner: &Inner,
    _id: PortalRequestId,
    _parent: &str,
    _kind: Kind,
    _cancel: async_channel::Receiver<()>,
) -> Result<Vec<PathBuf>, PortalError> {
    Err(PortalError::Unavailable)
}
