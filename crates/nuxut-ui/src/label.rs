use nuxut_core::{Color, Point, Size};
use nuxut_render::{Painter, TextLayout, TextStyle, TextSystem};

/// Önbellekli metin. Metin, stil ya da genişlik değişmedikçe yeniden şekillendirilmez.
/// `Debug` metni yazmaz (yalnızca uzunluk; değişmez 9).
#[derive(Clone)]
pub struct Label {
    text: String,
    style: TextStyle,
    max_width: Option<f32>,
    layout: Option<TextLayout>,
}

impl std::fmt::Debug for Label {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Label")
            .field("text", &crate::redact::Len(&self.text))
            .field("max_width", &self.max_width)
            .field("shaped", &self.layout.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[test]
fn label_debug_is_redacted() {
    let theme = nuxut_theme::Theme::default();
    let l = Label::new(crate::redact::SECRET, crate::typography::text_style(&theme, crate::typography::TextRole::Body));
    crate::redact::assert_redacted(&l);
}

impl Label {
    pub fn new(text: impl Into<String>, style: TextStyle) -> Self {
        Self { text: text.into(), style, max_width: None, layout: None }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn style(&self) -> &TextStyle {
        &self.style
    }

    /// Değiştiyse `true` (yeniden çizim gerekir).
    pub fn set_text(&mut self, text: &str) -> bool {
        if self.text == text {
            return false;
        }
        text.clone_into(&mut self.text);
        self.layout = None;
        true
    }

    pub fn set_style(&mut self, style: TextStyle) -> bool {
        if self.style == style {
            return false;
        }
        self.style = style;
        self.layout = None;
        true
    }

    pub fn max_width(&self) -> Option<f32> {
        self.max_width
    }

    /// `Some(w)`: `w`'de sarılır (ya da `max_lines` ile kesilir).
    pub fn set_max_width(&mut self, width: Option<f32>) -> bool {
        if self.max_width == width {
            return false;
        }
        self.max_width = width;
        self.layout = None;
        true
    }

    /// Önbellekteki layout (gerekirse şekillendirir).
    pub fn layout(&mut self, text: &mut TextSystem) -> &TextLayout {
        self.layout.get_or_insert_with(|| text.layout(&self.text, &self.style, self.max_width))
    }

    pub fn measure(&mut self, text: &mut TextSystem) -> Size {
        self.layout(text).size()
    }

    /// Çizim sırasında ölçüm (`draw` içinde `cx.text()` kullanılamaz).
    pub fn measure_in(&mut self, painter: &mut Painter) -> Size {
        self.measure(painter.text_system())
    }

    /// Sol-üst köşesi `origin` olacak şekilde çizer; layout önbellekte yoksa üretir.
    pub fn draw(&mut self, painter: &mut Painter, origin: Point, color: Color) {
        self.layout(painter.text_system());
        if let Some(layout) = &self.layout {
            painter.text(layout, origin, color);
        }
    }
}

// Erişilebilirlik (ADR-0011).
impl Label {
    /// `Role::Label`: görünen metin değer olarak (AT-SPI adı). `bounds`: çizildiği kutu
    /// (`Label` kendi konumunu tutmaz). Form satırında denetim `.labelled_by(id)` ile bağlanır.
    pub fn access<'t>(
        &self,
        tree: &'t mut crate::access::AccessTree,
        key: impl std::hash::Hash,
        bounds: nuxut_core::Rect,
    ) -> crate::access::AccessNode<'t> {
        tree.push(key, crate::access::Role::Label).value(&self.text).bounds(bounds)
    }
}
