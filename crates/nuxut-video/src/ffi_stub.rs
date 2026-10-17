//! Linux dışı platformlar (ADR-0003): libmpv yolu yok. Tipler `ffi` ile aynı arayüzü verir
//! ama kurulamaz; her kurucu `VideoError::Unavailable` döner.

use std::convert::Infallible;

use crate::VideoError;

const UNSUPPORTED: &str = "bu platformda video duvar kağıdı yok";

pub fn available() -> bool {
    false
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    None,
    Shutdown,
    FileLoaded,
    EndFile { reason: i32, error: i32 },
    VideoReconfig,
    PlaybackRestart,
    Other(i32),
}

#[derive(Debug)]
pub struct Mpv(Infallible);

impl Mpv {
    pub fn new(_options: &[(&str, &str)]) -> Result<Self, VideoError> {
        Err(VideoError::Unavailable(UNSUPPORTED.into()))
    }

    pub fn terminate_in_background(&mut self) {
        match self.0 {}
    }

    pub fn set_property(&self, _name: &str, _value: &str) -> Result<(), VideoError> {
        match self.0 {}
    }

    pub fn get_string(&self, _name: &str) -> Option<String> {
        match self.0 {}
    }

    pub fn get_f64(&self, _name: &str) -> Option<f64> {
        match self.0 {}
    }

    pub fn get_i64(&self, _name: &str) -> Option<i64> {
        match self.0 {}
    }

    pub fn command(&self, _args: &[&str]) -> Result<(), VideoError> {
        match self.0 {}
    }

    pub(crate) fn load(&self, _source: crate::source::MediaSource, _replace: bool) -> Result<(), VideoError> {
        match self.0 {}
    }

    pub(crate) fn single_media(&self) -> bool {
        match self.0 {}
    }

    pub fn wait_event(&self, _timeout: f64) -> Event {
        match self.0 {}
    }

    pub fn set_wakeup_callback(&mut self, _f: impl Fn() + Send + Sync + 'static) {
        match self.0 {}
    }

    pub fn create_render_context(&self, _update: impl Fn() + Send + Sync + 'static) -> Result<RenderContext, VideoError> {
        match self.0 {}
    }
}

#[derive(Debug)]
pub struct RenderContext(Infallible);

impl RenderContext {
    pub fn frame_ready(&mut self) -> bool {
        match self.0 {}
    }

    pub fn render(&mut self, _fbo: u32, _w: i32, _h: i32, _flip_y: bool) -> Result<(), VideoError> {
        match self.0 {}
    }
}
