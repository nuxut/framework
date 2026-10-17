#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("GL nesnesi oluşturulamadı: {0}")]
    Gl(String),
    #[error("{name} shader'ı derlenemedi:\n{log}")]
    Compile { name: &'static str, log: String },
    #[error("{name} programı bağlanamadı:\n{log}")]
    Link { name: &'static str, log: String },
    #[error("desteklenmeyen GL sürümü: {0} (OpenGL ES 3.0 gerekli)")]
    Unsupported(String),
}
