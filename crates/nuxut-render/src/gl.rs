//! GL backend uzantısı (ADR-0003 kural 3: çekirdek `Painter` API'sine GL tipi sızmaz).
//!
//! Dokuya doğrudan GL ile çizen dış üreticiler (ör. libmpv render API'si) bu modülü kullanır:
//! hedef `Painter::render_target` ile ayrılır, `PainterGlExt::gl_pass` ile doldurulur, sonra
//! normal `Painter::image` ile çizilir. Başka bir backend (wgpu/Metal) kendi uzantısını verir.
//!
//! GL nesnesi tutan dış değerler (mpv render context'i) herhangi bir thread'de, context güncel
//! değilken düşebilir; yok edilmeleri GL çağrısı yapar. [`GlGraveyard`] onları renderer'a
//! taşır: renderer her karenin başında (context güncel) ve kendi `Drop`'unda boşaltır.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use crate::{Painter, Texture};

/// GL thread'inde düşürülecek değer.
pub(crate) type GraveItem = Box<dyn Send>;

/// Renderer bunu `render_frame` içinde, yüklemelerden sonra ve çizimden önce, hedef dokuya
/// bağlı bir framebuffer'la çağırır; geçişten sonra kendi GL durumunu geri kurar
/// (framebuffer, viewport, program, VAO, ARRAY/ELEMENT buffer, doku birimi, scissor, stencil,
/// depth, cull, polygon offset, color mask, blend func/equation, UNPACK/PACK_ALIGNMENT,
/// UNPACK_ROW_LENGTH, PIXEL_UNPACK/PACK_BUFFER, birim 0/1 sampler ve dokuları).
pub trait GlPass: Send + Sync {
    /// `fbo`: hedef dokuya bağlı GL framebuffer adı; `size` fiziksel piksel. Context güncel.
    fn render(&self, gl: &glow::Context, fbo: u32, size: (u32, u32));
}

/// GL kaynağı tutan değerleri renderer'ın thread'inde, context güncelken düşürür. Ucuz
/// klonlanır, herhangi bir thread'den kullanılır.
#[derive(Clone)]
pub struct GlGraveyard(Sender<GraveItem>);

impl std::fmt::Debug for GlGraveyard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GlGraveyard")
    }
}

impl GlGraveyard {
    /// `value` bir sonraki karenin başında (ya da renderer düşerken) düşürülür. Renderer artık
    /// yoksa context de yoktur: değer **sızdırılır** (context'siz GL çağrısı bazı sürücülerde
    /// çöker; sızıntı süreç sonunda temizlenir).
    pub fn defer(&self, value: Box<dyn Send>) {
        if let Err(e) = self.0.send(value) {
            tracing::warn!("GL çöplüğü: renderer yok; nesne sızdırıldı");
            std::mem::forget(e.0);
        }
    }
}

/// `Painter` için GL'e özgü kanca.
pub trait PainterGlExt {
    /// `target`'a (bir `render_target`) bu karede `pass` ile çizilir. `render_target` olmayan
    /// dokuya geçiş atlanır (uyarı).
    fn gl_pass(&mut self, target: &Texture, pass: Arc<dyn GlPass>);
    /// Bu renderer'ın GL çöplüğü (bkz. [`GlGraveyard`]).
    fn gl_graveyard(&self) -> GlGraveyard;
}

impl PainterGlExt for Painter<'_> {
    fn gl_pass(&mut self, target: &Texture, pass: Arc<dyn GlPass>) {
        self.push_gl_pass(target, pass);
    }

    fn gl_graveyard(&self) -> GlGraveyard {
        // Renderer dışında kurulan Painter (testler) için kapalı kanal: defer sızdırır.
        GlGraveyard(self.graveyard_sender().unwrap_or_else(|| std::sync::mpsc::channel().0))
    }
}
