//! Nuxut UI çerçevesi. Uygulamalar yalnızca bu crate'e bağımlı olur.
//!
//! Temel tipler kökte (`nuxut::Rect`, `nuxut::Color`…); katmanlar modül olarak. Çekirdek
//! katmanlar (core, motion, config, theme, render, ui) her platformda derlenir; Wayland
//! runtime'ı (`App`, `Context`, `run`…) yalnızca Linux'tadır (ADR-0003).

#![forbid(unsafe_code)]

pub use nuxut_core::*;

/// Erişilebilirlik ağacı (ADR-0011): `AccessTree`, `AccessRequest`, kararlı kimlikler. AT-SPI adaptörü `a11y` özelliğiyle.
pub use nuxut_access as access;
pub use nuxut_config as config;
pub use nuxut_i18n as i18n;
pub use nuxut_locale as locale;
pub use nuxut_motion as motion;
pub use nuxut_render as render;
pub use nuxut_theme as theme;
pub use nuxut_ui as ui;
#[cfg(target_os = "linux")]
pub use nuxut_wayland as wayland;
/// Dosya seçici portalı (`portal` özelliği). Linux dışında istekler `Unavailable` ile biter.
#[cfg(feature = "portal")]
pub use nuxut_portal as portal;

pub use nuxut_core::input::{
    Button, CursorShape, Key, KeyEvent, KeyState, Modifiers, PointerEvent, PointerKind, PointerSource,
    TOUCH_SLOP, TOUCH_TAP_DELAY, TouchEvent, TouchId, TouchPhase,
};
pub use nuxut_motion::{Animated, Deform, Spring};
pub use nuxut_render::{
    BlobField, BlobShape, Border, FrameShape, Painter, RectStyle, ShadowStyle, TextLayout, TextStyle,
    TextSystem,
};
pub use nuxut_theme::Theme;
#[cfg(target_os = "linux")]
pub use nuxut_wayland::{
    ActivationRequestId, ActivationToken, Anchor, ExportRequestId, ToplevelHandle, App, Closed, ColorMatrix, ColorTransformError, ColorTransformState, Context, DataControlError,
    DataControlEvent, DataControlHandle, DataControlProtocol, DataOfferId, DataReadError, DataReadId, FocusGrabId, SetSelectionError, IdleInhibitors, IdleSupport, IdleWatchError, IdleWatchId, ImeEvent, KeyboardMode, Layer,
    LayerSurfaceSpec, OutputId, OutputInfo, OutputTarget, ResizeEdge, RunError, RunOptions, SurfaceId, SurroundingText,
    TextInputState, TextPurpose, TimerId, WindowSpec, WindowState, ipc, run, run_with,
};
/// Sanal klavye (ekran klavyesi; `Context::virtual_keyboard_*`).
#[cfg(target_os = "linux")]
pub use nuxut_wayland::VirtualKeyboardError;
/// Pencere listesi ve tek kare yakalama (`docs/design/capture.md`).
#[cfg(target_os = "linux")]
pub use nuxut_wayland::{
    CaptureBackend, CaptureError, CaptureId, CaptureOptions, CaptureSource, CaptureSupport,
    CapturedImage, ToplevelEvent, ToplevelId, ToplevelInfo, ToplevelListError,
};
/// Pano izleme (data-control) sabitleri ve okuma tamponu (`App::data_control_read`).
#[cfg(target_os = "linux")]
pub use nuxut_wayland::{
    MAX_MIME_LEN, MAX_MIMES, PASSWORD_HINT_MIME, Zeroizing, is_sensitive_mime, owner_mime,
};
