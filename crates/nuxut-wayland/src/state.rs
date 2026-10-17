//! sctk olay işleyicileri ve çizim döngüsü.

use std::time::Instant;

use nuxut_core::Point;
use nuxut_render::{FrameTarget, Renderer, Repaint};
use smithay_client_toolkit::compositor::{CompositorHandler, FrameCallbackData, SurfaceData};
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{self, KeyboardHandler, Keysym, RawModifiers};
use smithay_client_toolkit::seat::pointer::{self, PointerEventKind, PointerHandler, ThemeSpec};
use calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::seat::touch::TouchHandler;
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::session_lock::{
    SessionLock, SessionLockHandler, SessionLockSurface, SessionLockSurfaceConfigure,
};
use smithay_client_toolkit::shell::wlr_layer::{
    LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowHandler};
use smithay_client_toolkit::{delegate_dispatch2, delegate_registry, registry_handlers};
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface, wl_touch};
use wayland_client::{Connection, Proxy, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, wp_fractional_scale_v1,
    wp_fractional_scale_v1::WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};

use crate::App;
use crate::context::{Context, OutputEntry, RuntimeEvent};
use crate::egl::Egl;
use crate::surface::{Role, track_enter, track_leave};
use crate::types::{LockEnd, LockState, WindowState};
use crate::types::{OutputId, OutputInfo, SurfaceId};
use nuxut_core::input::{
    self, Button, Key, KeyEvent, KeyState, Modifiers, PointerEvent, PointerKind, SecretText,
    TouchEvent, TouchId, TouchPhase,
};

/// GPU tarafı: Context'ten ayrı alan ki `draw` sırasında renderer ile `&mut Context`
/// aynı anda ödünç alınabilsin.
pub(crate) struct Gfx {
    pub egl: Egl,
    /// İlk çizimde (context ilk kez bir yüzeye bağlanınca) yaratılır.
    pub renderer: Option<Renderer>,
    /// Runtime'ın renderer'dan bağımsız ham GL'i: kilit yüzeyini opaklaştırma ve hata ayıklama
    /// çerçevesi. Gerektiğinde (ilk kilit karesi ya da `debug_surfaces` ile açılışta) yüklenir.
    pub gl: Option<nuxut_render::glow::Context>,
    /// `NUXUT_DEBUG_SURFACES=1`: her yüzeyin sınırını boyar (şeffaf yüzey kapsamı hatalarını
    /// görmek için).
    pub debug_surfaces: bool,
    /// Yazılım sunumu (`shm` modülü): ilk çizimde, context yüzeysiz bağlanıp GL_RENDERER
    /// okununca karar verilir; `None` = henüz karar yok.
    pub software: Option<bool>,
}

pub(crate) struct State<A: App> {
    pub app: A,
    pub cx: Context<A>,
    pub gfx: Gfx,
    pub registry: RegistryState,
    pub output_state: OutputState,
    pub seat_state: SeatState,
    /// İşaretçinin koltuğu (xdg move/resize istekleri için).
    pointer_seat: Option<wl_seat::WlSeat>,
    /// Dokunmatik ekran ve koltuğu (dokunma serial'ı xdg move ve etkinleştirme için).
    touch: Option<wl_touch::WlTouch>,
    touch_seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Klavyenin koltuğu (etkinleştirme belirtecinin serial'i için).
    keyboard_seat: Option<wl_seat::WlSeat>,
    /// Gözlemci ikinci klavye: keymap'siz enter/tuşları izler (`keyfocus`).
    raw_keyboard: Option<wl_keyboard::WlKeyboard>,
    pub(crate) focus_gate: crate::keyfocus::FocusGate,
    /// Değiştiricisiz keysym çözümü (`KeyEvent::xkb`); keymap gelince derlenir.
    keysyms: Option<crate::keysym::KeysymResolver>,
    keyboard_focus: Option<SurfaceId>,
    modifiers: Modifiers,
    draw_queue: Vec<SurfaceId>,
    /// Açılış aşama süreleri (ilk karede bir kez loglanır).
    pub profile: crate::profile::StartupProfile,
    /// Katmanlı tema yükleyici; son geçerli paleti tutar.
    pub theme_loader: nuxut_theme::ThemeLoader,
}

impl<A: App> State<A> {
    pub(crate) fn new(
        app: A,
        cx: Context<A>,
        gfx: Gfx,
        registry: RegistryState,
        output_state: OutputState,
        seat_state: SeatState,
    ) -> Self {
        Self {
            app,
            cx,
            gfx,
            registry,
            output_state,
            seat_state,
            pointer_seat: None,
            touch: None,
            touch_seat: None,
            keyboard: None,
            keyboard_seat: None,
            raw_keyboard: None,
            focus_gate: Default::default(),
            keysyms: None,
            keyboard_focus: None,
            modifiers: Modifiers::default(),
            draw_queue: Vec::new(),
            theme_loader: nuxut_theme::ThemeLoader::new(),
            profile: crate::profile::StartupProfile::new(),
        }
    }

    /// Her olay turunun sonunda: bekleyen yüzey yıkımları, sonra istenen kareler.
    pub(crate) fn after_dispatch(&mut self) {
        self.reconcile_keyboard();
        // Context'in kuyruğa aldığı olaylar (ör. kilit yüzeyi yaratıldı) çizimden önce.
        while !self.cx.pending.is_empty() {
            for event in std::mem::take(&mut self.cx.pending) {
                match event {
                    RuntimeEvent::LockSurfaceAdded(id, output) => {
                        if self.cx.surfaces.contains_key(&id) {
                            self.app.lock_surface_added(&mut self.cx, id, output);
                        }
                    }
                    RuntimeEvent::LockFinished(end) => self.app.lock_finished(&mut self.cx, end),
                    RuntimeEvent::ActivationToken(id, token) => {
                        self.app.activation_token(&mut self.cx, id, token);
                    }
                    RuntimeEvent::Paste(id, result) => self.app.paste(&mut self.cx, id, result),
                    RuntimeEvent::ToplevelExported(id, surface, handle) => {
                        self.app.toplevel_exported(&mut self.cx, id, surface, handle);
                    }
                    RuntimeEvent::FocusGrabCleared(id) => {
                        self.app.focus_grab_cleared(&mut self.cx, id);
                    }
                    RuntimeEvent::ColorTransformBlocked => {
                        self.app.color_transform_blocked(&mut self.cx);
                    }
                    // Kuyruktayken yerine yenisi gelen (ya da durdurulan) teklif ve izlemesi
                    // kapatılan birincil seçimin olayı düşülür.
                    RuntimeEvent::DataControl(event) => {
                        if self.cx.data_control_deliverable(&event) {
                            self.app.data_control(&mut self.cx, event);
                        }
                    }
                    RuntimeEvent::DataRead(id, outcome) => {
                        self.app.data_control_read(&mut self.cx, id, outcome.0);
                    }
                    RuntimeEvent::Toplevel(event) => self.app.toplevel(&mut self.cx, event),
                    RuntimeEvent::Captured(id, result) => {
                        self.app.captured(&mut self.cx, id, result);
                    }
                    // Kuyruktayken `idle_unwatch` edilen izlemenin olayı düşülür.
                    RuntimeEvent::Idle(id, idle) => {
                        if self.cx.idle_watch_alive(id) {
                            self.app.idle(&mut self.cx, id, idle);
                        }
                    }
                }
            }
        }
        self.reap_surfaces();
        self.draw_queue.clear();
        self.draw_queue.extend(
            self.cx
                .surfaces
                .iter()
                .filter(|(_, s)| s.ready_to_draw())
                .map(|(id, _)| *id),
        );
        // Açılış: uygulama yüzeylerini yarattı ama configure henüz gelmedi. İstekleri gönder ve
        // compositor configure'ı hazırlarken renderer'ı (shader derleme) yüzeysiz context'te kur.
        if self.gfx.renderer.is_none() && !self.cx.surfaces.is_empty() && self.draw_queue.is_empty()
        {
            if let Err(e) = self.cx.conn.flush() {
                tracing::debug!("erken flush: {e}");
            }
            if self.gfx.egl.make_current_surfaceless().is_ok()
                && let Err(e) = ensure_renderer(&mut self.gfx, &mut self.profile)
            {
                tracing::error!("renderer başlatılamadı: {e}");
                self.cx.quit = true;
                return;
            }
        }
        if self.draw_queue.is_empty() {
            return;
        }
        // Bu turdaki tüm yüzeyler aynı saati görür.
        let now = Instant::now();
        let queue = std::mem::take(&mut self.draw_queue);
        for &id in &queue {
            self.draw(id, now);
        }
        self.draw_queue = queue;
        // draw içinde destroy (release'te yok sayılır) ya da başarısız yüzey oluşturma.
        self.reap_surfaces();
    }

    fn reap_surfaces(&mut self) {
        // Odaktaki yüzey yok edildiyse odağı sessizce düşür (uygulamaya çağrı yok).
        if self
            .keyboard_focus
            .is_some_and(|id| !self.cx.surfaces.contains_key(&id))
        {
            self.keyboard_focus = None;
        }
        if self.cx.graveyard.is_empty() {
            return;
        }
        for mut entry in self.cx.graveyard.drain(..) {
            if let Some(gpu) = entry.gpu.take() {
                self.gfx.egl.destroy_surface(gpu);
            }
            if let Some(target) = entry.shm.take() {
                // FBO silinirken context güncel olmalı; bağlanamazsa GL nesneleri sürücüye kalır.
                let current = self.gfx.egl.make_current_surfaceless().is_ok();
                target.destroy(
                    self.gfx
                        .renderer
                        .as_ref()
                        .filter(|_| current)
                        .map(Renderer::gl),
                );
            }
        }
        // Sızıntı denetimi için (hotplug testleri): yalnızca bir şey yok edildiğinde.
        let (textures, texture_bytes) = self
            .gfx
            .renderer
            .as_ref()
            .map_or((0, 0), Renderer::texture_stats);
        tracing::debug!(
            surfaces = self.cx.surfaces.len(),
            egl_surfaces = self.gfx.egl.live_surfaces(),
            textures,
            texture_bytes,
            "yüzeyler yok edildi"
        );
    }

    /// Tüm yüzeyleri ve EGL'yi Wayland nesnelerinden önce yok eder.
    pub(crate) fn shutdown(&mut self) {
        self.cx.focus_grab_shutdown();
        self.cx.color_transform_shutdown();
        self.cx.capture_shutdown();
        self.cx.toplevels_shutdown();
        let ids: Vec<SurfaceId> = self.cx.surfaces.keys().copied().collect();
        for id in ids {
            if let Some(entry) = self.cx.surfaces.remove(&id) {
                self.cx.graveyard.push(entry);
            }
        }
        self.reap_surfaces();
        // Renderer Drop'ta GL nesnelerini siler: context güncel olmalı.
        if self.gfx.renderer.is_some()
            && let Err(e) = self.gfx.egl.make_current_surfaceless()
        {
            tracing::warn!(
                "kapanışta context bağlanamadı ({e}); GL nesneleri sürücüye bırakılıyor"
            );
        }
        self.gfx.renderer = None;
        self.gfx.gl = None;
    }

    /// Sunum yolu kararı (ilk çizimde bir kez): context yüzeysiz bağlanır, renderer kurulur,
    /// GL_RENDERER bir yazılım rasterleştiricisiyse (ya da `NUXUT_SOFTWARE_PRESENT=1`) shm
    /// sunumu. Yüzeysiz context ya da `wl_shm` yoksa EGL pencere yüzeyi. `None`: renderer
    /// kurulamadı (çıkılıyor).
    fn decide_software(&mut self) -> Option<bool> {
        let env = std::env::var("NUXUT_SOFTWARE_PRESENT").ok();
        if env.as_deref().map(str::trim) == Some("0") || self.cx.protocols.shm.is_none() {
            self.gfx.software = Some(false);
            return Some(false);
        }
        if let Err(e) = self.gfx.egl.make_current_surfaceless() {
            tracing::debug!("yüzeysiz context yok ({e}); EGL pencere yüzeyi kullanılacak");
            self.gfx.software = Some(false);
            return Some(false);
        }
        if let Err(e) = ensure_renderer(&mut self.gfx, &mut self.profile) {
            tracing::error!("renderer başlatılamadı: {e}");
            self.cx.quit = true;
            return None;
        }
        let name = self.gfx.renderer.as_ref().map_or_else(String::new, |r| {
            use nuxut_render::glow::{HasContext, RENDERER};
            // SAFETY: context güncel (yüzeysiz); glGetString statik dizge döndürür.
            unsafe { r.gl().get_parameter_string(RENDERER) }
        });
        let software = crate::shm::wanted(&name, env.as_deref());
        tracing::info!(
            software,
            "sunum yolu: {}",
            if software {
                "shm (yazılım)"
            } else {
                "EGL pencere yüzeyi"
            }
        );
        self.gfx.software = Some(software);
        Some(software)
    }

    fn draw(&mut self, id: SurfaceId, now: Instant) {
        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        entry.wants_frame = false;
        let size_px = entry.physical_size();
        let gl_size = (size_px.0 as i32, size_px.1 as i32);
        let scale = entry.scale;

        let software = match self.gfx.software {
            Some(s) => s,
            None => match self.decide_software() {
                Some(s) => s,
                None => return,
            },
        };
        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        let mut fbo_age = 0;
        if software {
            if let Err(e) = self.gfx.egl.make_current_surfaceless() {
                tracing::warn!(
                    ?id,
                    "eglMakeCurrent (yüzeysiz) başarısız ({e}); kare atlandı"
                );
                return;
            }
            let Some(renderer) = self.gfx.renderer.as_ref() else {
                return;
            };
            match entry
                .shm
                .get_or_insert_with(crate::shm::ShmSurface::new)
                .bind(renderer.gl(), size_px)
            {
                Ok(age) => fbo_age = age,
                Err(e) => {
                    tracing::error!(?id, "yazılım sunumu hedefi yaratılamadı: {e}");
                    return;
                }
            }
        } else if entry.gpu.is_none() {
            match self
                .gfx
                .egl
                .create_surface(entry.role.wl_surface(), gl_size)
            {
                Ok(gpu) => entry.gpu = Some(gpu),
                Err(e) => {
                    tracing::error!(?id, "EGL yüzeyi yaratılamadı: {e}");
                    return;
                }
            }
        }
        if !software {
            let Some(gpu) = entry.gpu.as_mut() else {
                return;
            };
            gpu.resize(gl_size);
            if let Err(e) = self.gfx.egl.make_current(gpu) {
                tracing::warn!(?id, "eglMakeCurrent başarısız ({e}); kare atlandı");
                return;
            }
            if gpu.take_first_use() {
                self.gfx.egl.disable_vsync();
            }
            if let Err(e) = ensure_renderer(&mut self.gfx, &mut self.profile) {
                tracing::error!("renderer başlatılamadı: {e}");
                self.cx.quit = true;
                return;
            }
        }

        entry.apply_scale();
        // Bekleyen layer yerleşimi bu karenin commit'iyle gider (çağrılar kareye birleşir).
        if let Some(_p) = entry.apply_placement() {
            #[cfg(feature = "a11y")]
            self.cx.a11y.set_place(id, crate::a11y::LayerPlace { anchor: _p.anchor, margin: _p.insets() });
        }
        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        // Callback swap'in (ya da shm sunumunun) yaptığı commit'e bağlanır.
        let wl_surface = entry.role.wl_surface().clone();
        wl_surface.frame(&self.cx.qh, FrameCallbackData(wl_surface.clone()));
        entry.callback_pending = true;

        let buffer_age = if software {
            fbo_age
        } else {
            self.cx
                .surfaces
                .get(&id)
                .and_then(|e| e.gpu.as_ref())
                .map_or(0, |gpu| self.gfx.egl.buffer_age(gpu))
        };
        let mut history = self
            .cx
            .surfaces
            .get_mut(&id)
            .map(|e| std::mem::take(&mut e.damage))
            .unwrap_or_default();
        let t = Instant::now();
        let loading = self.cx.text.is_none();
        let mut text = self.cx.take_text();
        if loading {
            // Arka planda başlatıldıysa bu yalnızca kalan bekleme süresidir.
            self.profile.mark_span("TextSystem bekleme", t);
        }
        let mut repaint = Repaint::Full;
        // Kare zamanlaması (`RUST_LOG=nuxut::frame=trace`): kayıt (App::draw), GL gönderimi,
        // swap. llvmpipe'ta rasterleştirme swap'te beklenir.
        let mut record = std::time::Duration::ZERO;
        let t_render = Instant::now();
        if let Some(renderer) = self.gfx.renderer.as_mut() {
            let (app, cx) = (&mut self.app, &mut self.cx);
            let target = FrameTarget {
                size_px,
                scale,
                buffer_age,
            };
            cx.in_draw = true;
            repaint = renderer.render_frame(&mut text, &mut history, target, |painter| {
                let t = Instant::now();
                app.draw(cx, id, now, painter);
                record = t.elapsed();
            });
            cx.in_draw = false;
        }
        let render = t_render.elapsed();
        let redrawn = history.last_region();
        self.cx.text = Some(text);
        if self.cx.is_lock_surface(id) {
            // Savunma derinliği (kural 2): uygulama yarı saydam boyasa da kilit yüzeyi opak.
            let egl = &self.gfx.egl;
            force_opaque(self.gfx.gl.get_or_insert_with(|| egl.load_gl()));
        }
        if self.gfx.debug_surfaces
            && let Some(gl) = &self.gfx.gl
        {
            debug_outline(gl, size_px);
            repaint = Repaint::Full;
        }

        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        entry.damage = history;
        // Kare başına yükleme bütçesi dolduysa kalan dokular sonraki karelerde yüklenir:
        // uygulama bir şey yapmadan sürsün diye bu yüzeye bir kare daha iste. Bütçe dolmadıkça
        // ek kare yok (boşta iş yok).
        if self
            .gfx
            .renderer
            .as_ref()
            .is_some_and(Renderer::has_pending_uploads)
        {
            tracing::trace!(?id, "bekleyen doku yüklemesi; bir kare daha");
            entry.wants_frame = true;
        }
        let t_swap = Instant::now();
        let mut swapped = !repaint.is_nothing();
        let result: Result<(), String> = if software {
            let gl = self.gfx.renderer.as_ref().map(Renderer::gl);
            match (entry.shm.as_mut(), gl, self.cx.protocols.shm.as_ref()) {
                (Some(target), Some(gl), Some(shm)) => {
                    match target.present(gl, entry.role.wl_surface(), shm, &self.cx.qh, repaint) {
                        Ok(crate::shm::Presented::Buffer) => Ok(()),
                        Ok(crate::shm::Presented::Unchanged) => {
                            swapped = false;
                            Ok(())
                        }
                        Ok(crate::shm::Presented::Deferred) => {
                            tracing::trace!(?id, "shm tamponlarının hepsi meşgul; bir kare daha");
                            swapped = false;
                            entry.wants_frame = true;
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
                _ => Err("shm sunum hedefi yok".into()),
            }
        } else {
            let Some(gpu) = entry.gpu.as_ref() else {
                return;
            };
            match repaint {
                // Hiçbir şey değişmedi: tampon göndermeden commit et ki istenen frame callback
                // ve bekleyen yüzey durumu (viewport vb.) yine de compositor'a ulaşsın.
                Repaint::Partial(r) if r.is_empty() => {
                    entry.role.wl_surface().commit();
                    Ok(())
                }
                Repaint::Partial(r) => {
                    let mut rects = [[0i32; 4]; nuxut_render::MAX_DAMAGE_RECTS];
                    for (d, p) in rects.iter_mut().zip(r.rects()) {
                        *d = p.to_egl(size_px.1);
                    }
                    self.gfx
                        .egl
                        .swap_buffers_with_damage(gpu, &rects[..r.len()])
                }
                Repaint::Full => self.gfx.egl.swap_buffers(gpu),
            }
            .map_err(|e| format!("eglSwapBuffers: {e}"))
        };
        tracing::trace!(
            target: "nuxut::frame",
            surface = ?id,
            size = ?size_px,
            age = buffer_age,
            software,
            damage = ?repaint,
            redrawn = ?redrawn,
            record_us = record.as_micros() as u64,
            gl_us = render.saturating_sub(record).as_micros() as u64,
            swap_us = t_swap.elapsed().as_micros() as u64,
            "kare"
        );
        let first_buffer = result.is_ok() && !entry.has_buffer;
        let result = {
            if result.is_ok() {
                self.profile.first_frame();
                entry.has_buffer = true;
                if swapped {
                    entry.presentation.buffer_committed();
                }
            }
            result
        };
        if let Err(e) = result {
            tracing::warn!(?id, "kare gönderilemedi: {e}");
            // Commit olmadıysa callback hiç gelmez; yüzeyi kilitleme.
            entry.callback_pending = false;
        }
        if first_buffer {
            // Yüzey artık map edildi: onu bekleyen odak kapmaları kurulur (protokol isteği;
            // uygulamaya çağrı yok).
            self.cx.focus_grab_surface_mapped(id);
        }
    }

    /// `palette.toml` değişti: tema iki katmanıyla yeniden yüklenir.
    pub(crate) fn palette_event(&mut self) {
        if self.cx.palette_changed() {
            tracing::debug!("palet değişti");
            self.reload_theme();
        }
    }

    /// Kullanıcı çeviri dizininde olay: birleştirme zamanlayıcısı kurulur (yeniden kurma
    /// `i18n_timer`'da, tek sefer).
    pub(crate) fn i18n_event(&mut self) {
        self.cx.i18n_events();
    }

    /// Çalışma sırasında yeni bir çeviri domain'i kaydedildi: izleyici kümesi güncellenir.
    pub(crate) fn i18n_registered(&mut self) {
        self.cx.sync_i18n_watcher();
    }

    /// Birleştirme süresi doldu: katalog girdisi gerçekten değiştiyse katalog yeniden kurulur
    /// ve `App::locale_changed` gider; aynı içerikse iş yok.
    pub(crate) fn i18n_timer(&mut self) -> calloop::timer::TimeoutAction {
        match self.cx.i18n_poll() {
            Err(deadline) => calloop::timer::TimeoutAction::ToInstant(deadline),
            Ok(changed) => {
                if changed {
                    tracing::debug!("kullanıcı çeviri dosyaları değişti");
                    nuxut_i18n::reload();
                    self.app.locale_changed(&mut self.cx);
                }
                calloop::timer::TimeoutAction::Drop
            }
        }
    }

    pub(crate) fn config_event(&mut self) {
        for name in self.cx.changed_configs() {
            tracing::debug!(name, "config değişti");
            if name == nuxut_theme::CONFIG_NAME {
                self.reload_theme();
            } else {
                self.app.config_changed(&mut self.cx, &name);
            }
        }
    }

    fn reload_theme(&mut self) {
        match self.theme_loader.load() {
            // Uyarıları nuxut-theme kendisi loglar.
            Ok(loaded) => {
                if loaded.value != self.cx.theme {
                    let visual = !loaded.value.same_visuals(&self.cx.theme);
                    let locale = loaded.value.locale != self.cx.theme.locale;
                    self.cx.theme = loaded.value;
                    if visual {
                        self.app.theme_changed(&mut self.cx);
                    }
                    // [locale]: yeni katalog kurulur ve süreç geneli yapılır (ADR-0010);
                    // yalnızca değişimde, boşta iş yok.
                    if locale && nuxut_i18n::configure(&self.cx.theme.locale) {
                        tracing::debug!("locale değişti");
                        self.app.locale_changed(&mut self.cx);
                    }
                }
            }
            Err(e) => tracing::warn!(
                error = %e.log_summary(),
                "framework.toml yüklenemedi, önceki tema korunuyor"
            ),
        }
    }

    fn surface_id(surface: &wl_surface::WlSurface) -> Option<SurfaceId> {
        surface.data::<SurfaceData<SurfaceId>>().map(|d| *d.data())
    }

    fn notify_resized(&mut self, id: SurfaceId) {
        let Some(entry) = self.cx.surfaces.get(&id) else {
            return;
        };
        let (size, scale) = (entry.logical_size(), entry.scale);
        self.app.surface_resized(&mut self.cx, id, size, scale);
    }

    fn set_scale(&mut self, id: SurfaceId, scale: f32) {
        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        if (entry.scale - scale).abs() < 1e-4 {
            return;
        }
        tracing::debug!(?id, scale, "yüzey ölçeği değişti");
        entry.scale = scale;
        entry.scale_dirty = true;
        entry.wants_frame = true;
        if entry.configured {
            self.notify_resized(id);
        }
    }

    /// Çıkışın `wl_output.transform` kodu (yakalamanın wlr yedeği dikleştirmek için).
    fn output_transform(&self, output: &wl_output::WlOutput) -> u32 {
        self.output_state
            .info(output)
            .map_or(0, |i| u32::from(i.transform))
    }

    fn output_info(&self, output: &wl_output::WlOutput) -> Option<OutputInfo> {
        let info = self.output_state.info(output)?;
        let (x, y) = info.logical_position.unwrap_or(info.location);
        let mode = info.modes.iter().find(|m| m.current);
        let mode_size = mode.map_or((0, 0), |m| (m.dimensions.0.max(0) as u32, m.dimensions.1.max(0) as u32));
        let physical_size = oriented_mode_size(mode_size, info.transform);
        let (w, h) = info.logical_size.unwrap_or_else(|| {
            let s = info.scale_factor.max(1) as u32;
            ((physical_size.0 / s) as i32, (physical_size.1 / s) as i32)
        });
        Some(OutputInfo {
            id: OutputId(info.id),
            name: info.name.clone().unwrap_or_default(),
            description: info.description.clone().unwrap_or_default(),
            position: Point::new(x as f32, y as f32),
            size: nuxut_core::Size::new(w as f32, h as f32),
            scale: info.scale_factor.max(1) as f32,
            refresh: mode.map_or(0.0, |m| m.refresh_rate as f32 / 1000.0),
            mode_size,
            physical_size,
        })
    }

    fn key_event(&mut self, event: keyboard::KeyEvent, state: KeyState) {
        let keyboard::KeyEvent {
            raw_code,
            keysym,
            utf8,
            ..
        } = event;
        // xkb'nin ürettiği metin kopyalanmadan SecretText'e taşınır: aşağıdaki her dönüş
        // yolunda (odak yok, bırakma, kontrol karakteri) düşerken sıfırlanır.
        let utf8 = utf8.map(SecretText::from);
        let Some(id) = self
            .keyboard_focus
            .filter(|id| self.cx.surfaces.contains_key(id))
            .filter(|id| self.cx.input_allowed(*id))
        else {
            return;
        };
        let text = if state == KeyState::Released {
            None
        } else {
            input::printable_secret(utf8)
        };
        let key = match Key::from_keysym(keysym.raw()) {
            // Eski (Latin-2 vb.) keysym'ler: xkb'nin karakter tablosuna sor.
            Key::Other(raw) => keysym
                .key_char()
                .filter(|c| !c.is_control())
                .map_or(Key::Other(raw), Key::Char),
            key => key,
        };
        let event = KeyEvent {
            key,
            state,
            text,
            modifiers: self.modifiers,
            xkb: self
                .keysyms
                .as_ref()
                .map(|r| r.resolve(raw_code, keysym.raw())),
        };
        self.app.keyboard(&mut self.cx, id, event);
    }
}

/// Renderer'ı (bir kez) yaratır. GL context güncel olmalı (bir yüzeye ya da yüzeysiz).
fn ensure_renderer(
    gfx: &mut Gfx,
    profile: &mut crate::profile::StartupProfile,
) -> Result<(), String> {
    if gfx.renderer.is_some() {
        return Ok(());
    }
    let gl = gfx.egl.load_gl();
    {
        use nuxut_render::glow::{HasContext, RENDERER, VERSION};
        // SAFETY: context güncel; glGetString sabit, statik ömürlü dizge döndürür.
        let (renderer, version) = unsafe {
            (
                gl.get_parameter_string(RENDERER),
                gl.get_parameter_string(VERSION),
            )
        };
        tracing::info!(renderer, version, "GL context hazır");
    }
    let t = Instant::now();
    let renderer = Renderer::new(gl);
    profile.mark_span("Renderer::new", t);
    gfx.renderer = Some(renderer.map_err(|e| e.to_string())?);
    if gfx.debug_surfaces {
        gfx.gl = Some(gfx.egl.load_gl());
    }
    Ok(())
}
/// Çizilmiş karenin alfa kanalını tümüyle 1 yapar; renkler dokunulmaz. Renkler önceden
/// çarpılmış olduğundan sonuç, karenin opak siyah üzerine bindirilmiş hâlidir (hiç çizilmemiş
/// yer opak siyah). Kısmi yeniden çizimde hasar dışı bölge zaten önceki karelerden opaktır.
fn force_opaque(gl: &nuxut_render::glow::Context) {
    use nuxut_render::glow::{COLOR_BUFFER_BIT, HasContext, SCISSOR_TEST};
    // SAFETY: context bu yüzeye bağlı ve güncel; yalnızca durum bayrakları ve clear.
    unsafe {
        gl.disable(SCISSOR_TEST);
        gl.color_mask(false, false, false, true);
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(COLOR_BUFFER_BIT);
        gl.color_mask(true, true, true, true);
    }
}

/// Yüzey sınırına 2px eflatun çerçeve (yalnızca hata ayıklama).
fn debug_outline(gl: &nuxut_render::glow::Context, (w, h): (u32, u32)) {
    use nuxut_render::glow::HasContext;
    let (w, h) = (w as i32, h as i32);
    let t = 2.min(w).min(h);
    // SAFETY: context bu yüzeye bağlı ve güncel; scissor kutuları framebuffer içinde.
    unsafe {
        gl.enable(nuxut_render::glow::SCISSOR_TEST);
        gl.clear_color(0.8, 0.0, 0.8, 0.8);
        for (x, y, bw, bh) in [
            (0, 0, w, t),
            (0, h - t, w, t),
            (0, 0, t, h),
            (w - t, 0, t, h),
        ] {
            gl.scissor(x, y, bw, bh);
            gl.clear(nuxut_render::glow::COLOR_BUFFER_BIT);
        }
        gl.disable(nuxut_render::glow::SCISSOR_TEST);
    }
}

impl<A: App> CompositorHandler for State<A> {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        let Some(id) = Self::surface_id(surface) else {
            return;
        };
        // Kesirli ölçek yolu varsa tamsayı ölçek yok sayılır.
        if self
            .cx
            .surfaces
            .get(&id)
            .is_some_and(|s| s.fractional.is_none())
        {
            self.set_scale(id, factor.max(1) as f32);
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        // Yok edilmiş yüzey haritada değil: olay yok.
        let Some(id) = Self::surface_id(surface) else {
            return;
        };
        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        entry.callback_pending = false;
        if entry.presentation.frame_done() {
            tracing::debug!(?id, "yüzey ilk kez sunuldu");
            self.app.surface_presented(&mut self.cx, id);
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        // Ölçeği sctk (`scale_factor_changed`) ve kesirli ölçek nesnesi ayrıca getirir; burada
        // yalnızca çıkışı compositor'a bırakılmış yüzeyin çıkışı izlenir.
        let (Some(id), Some(out)) = (Self::surface_id(surface), self.cx.output_id_of(output))
        else {
            return;
        };
        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        if track_enter(&mut entry.output, entry.follows_enter, out) {
            tracing::debug!(?id, output = ?out, "compositor yüzeyin çıkışını seçti");
            self.app.surface_output_changed(&mut self.cx, id, Some(out));
        }
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        let (Some(id), Some(out)) = (Self::surface_id(surface), self.cx.output_id_of(output))
        else {
            return;
        };
        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        if track_leave(&mut entry.output, entry.follows_enter, out) {
            tracing::debug!(?id, output = ?out, "yüzey çıkışından ayrıldı");
            self.app.surface_output_changed(&mut self.cx, id, None);
        }
    }
}

impl<A: App> OutputHandler for State<A> {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let Some(info) = self.output_info(&output) else {
            return;
        };
        tracing::debug!(?info, "çıkış eklendi");
        let transform = self.output_transform(&output);
        self.cx.outputs.insert(
            info.id,
            OutputEntry {
                wl: output,
                scale: info.scale as i32,
                transform,
            },
        );
        self.app.output_added(&mut self.cx, &info);
        // Kilit istendiyse/kurulduysa yeni çıkışa da kilit yüzeyi (kural 5).
        if self.cx.lock.is_some() && self.cx.create_lock_surface(info.id).is_some() {
            tracing::info!(state = ?self.cx.lock_state(), "kilit varken çıkış eklendi; kilit yüzeyi yaratıldı");
        }
    }

    fn update_output(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let Some(info) = self.output_info(&output) else {
            return;
        };
        let transform = self.output_transform(&output);
        if let Some(entry) = self.cx.outputs.get_mut(&info.id) {
            entry.scale = info.scale as i32;
            entry.transform = transform;
        }
        self.app.output_changed(&mut self.cx, &info);
    }

    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let Some(id) = self
            .cx
            .outputs
            .iter()
            .find(|(_, o)| o.wl == output)
            .map(|(id, _)| *id)
        else {
            return;
        };
        self.cx.outputs.remove(&id);
        self.cx.color_transform_forget_output(id);
        tracing::debug!(?id, "çıkış kaldırıldı");
        // Bu çıkışın kilit yüzeyi (varsa) kaldırılır; protokol bunu önerir.
        let lock_ids: Vec<SurfaceId> = self
            .cx
            .surfaces
            .iter()
            .filter(|(_, s)| s.output == Some(id) && matches!(s.role, Role::Lock(_)))
            .map(|(sid, _)| *sid)
            .collect();
        for sid in lock_ids {
            self.cx.focus_grab_forget_surface(sid);
            if let Some(entry) = self.cx.surfaces.remove(&sid) {
                self.cx.graveyard.push(entry);
            }
            self.cx.ime.forget(sid);
            self.cx.cursor.tracker.forget(sid);
            self.cx.touch.surface_removed(sid);
        }
        self.app.output_removed(&mut self.cx, id);
    }
}

impl<A: App> WindowHandler for State<A> {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window) {
        if let Some(id) = self.cx.window_id_of(window) {
            self.app.close_requested(&mut self.cx, id);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        let Some(id) = self.cx.window_id_of(window) else {
            return;
        };
        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        // Compositor boyut bırakırsa (None): son boyut, ilk configure'da istenen boyut.
        let fallback = if entry.configured {
            entry.size
        } else {
            entry.requested
        };
        let size = (
            configure.new_size.0.map_or(fallback.0, |w| w.get()),
            configure.new_size.1.map_or(fallback.1, |h| h.get()),
        );
        let state = WindowState {
            maximized: configure.is_maximized(),
            fullscreen: configure.is_fullscreen(),
            activated: configure.is_activated(),
            tiled: configure.is_tiled(),
        };
        let first = !entry.configured;
        if first {
            self.profile.mark_once("ilk configure");
        }
        let resized = first || size != entry.size;
        entry.size = (size.0.max(1), size.1.max(1));
        entry.configured = true;
        entry.wants_frame = true;
        let state_changed = match &mut entry.role {
            Role::Window { state: old, .. } => std::mem::replace(old, state) != state,
            Role::Layer(_) | Role::Lock(_) => false,
        };
        if resized {
            entry.scale_dirty = true;
            self.notify_resized(id);
        }
        if state_changed || first {
            self.app.window_state_changed(&mut self.cx, id, state);
        }
    }
}

impl<A: App> SessionLockHandler for State<A> {
    fn locked(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
        let Some(lock) = self.cx.lock.as_mut() else {
            return;
        };
        lock.state = LockState::Locked;
        tracing::info!("oturum kilitlendi (Locked)");
        self.app.session_locked(&mut self.cx);
    }

    fn finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
        let Some(lock) = self.cx.lock.take() else {
            return;
        };
        // Protokol: `locked` alındıysa `unlock_and_destroy`, alınmadıysa `destroy` (Drop).
        let end = if lock.state == LockState::Locked {
            lock.session.unlock();
            LockEnd::CompositorEnded
        } else {
            LockEnd::Refused
        };
        drop(lock);
        self.cx.remove_lock_surfaces();
        tracing::info!(?end, "oturum kilidi bitti");
        self.app.lock_finished(&mut self.cx, end);
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.cx.lock_surface_id_of(&surface) else {
            return;
        };
        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        if !entry.configured {
            self.profile.mark_once("ilk configure");
        }
        // Kilit yüzeyinin boyutu compositor'ındır (çıkışın tamamı).
        let size = (configure.new_size.0.max(1), configure.new_size.1.max(1));
        let changed = !entry.configured || size != entry.size;
        entry.size = size;
        entry.configured = true;
        // İlk configure'da hemen kare (kural 2): compositor ilk tampona dek siyah gösterir.
        entry.wants_frame = true;
        if changed {
            entry.scale_dirty = true;
            self.notify_resized(id);
        }
    }
}

impl<A: App> LayerShellHandler for State<A> {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        // Compositor yüzeyi kapattı (çoğunlukla çıkış gitti). Uygulama `output_removed` alır.
        if let Some(id) = self.cx.surface_id_of(layer) {
            tracing::debug!(?id, "compositor yüzeyi kapattı");
            self.cx.focus_grab_forget_surface(id);
            if let Some(entry) = self.cx.surfaces.remove(&id) {
                self.cx.graveyard.push(entry);
                self.cx.ime.forget(id);
                self.cx.cursor.tracker.forget(id);
                self.cx.touch.surface_removed(id);
            }
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.cx.surface_id_of(layer) else {
            return;
        };
        let Some(entry) = self.cx.surfaces.get_mut(&id) else {
            return;
        };
        let pick = |given: u32, requested: u32| if given > 0 { given } else { requested.max(1) };
        let size = (
            pick(configure.new_size.0, entry.requested.0),
            pick(configure.new_size.1, entry.requested.1),
        );
        if !entry.configured {
            self.profile.mark_once("ilk configure");
        }
        let changed = !entry.configured || size != entry.size;
        entry.size = size;
        entry.configured = true;
        // Configure sonrası mutlaka bir commit (ilk configure'da yüzeyi eşler). Boyut aynıysa ve
        // yüzeyin tamponu varsa içerik geçerlidir: ack'i (sctk gönderdi) bağlamak için tampon
        // göndermeden commit yeter. Compositor'lar başka bir katman yüzeyi durum değiştirince
        // (ör. panel açılınca klavye kipi) aynı boyutla tüm katmanları yeniden configure eder;
        // eskiden her biri (duvar kağıdı dahil) tam yüzey yeniden çiziliyordu.
        if changed || !entry.has_buffer {
            entry.wants_frame = true;
        } else {
            entry.role.wl_surface().commit();
        }
        if changed {
            entry.scale_dirty = true;
            self.notify_resized(id);
        }
    }
}

impl<A: App> SeatHandler for State<A> {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        // Açılıştaki koltuklar burada gelmez (`run_with` verir); bu, sonradan ilan edilen.
        self.cx.data_control_seat_added(seat.clone());
        self.cx.idle_seat_added(seat);
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer if self.cx.cursor.pointer.is_none() => {
                let Some(shm) = self.cx.protocols.shm.clone() else {
                    tracing::warn!("wl_shm yok; işaretçi alınmadı");
                    return;
                };
                // İmleç: cursor-shape varsa onunla, yoksa xcursor temasıyla (ayrı yüzey).
                let cursor_surface = self.cx.protocols.compositor.create_surface(qh);
                match self.seat_state.get_pointer_with_theme::<Self, ()>(
                    qh,
                    &seat,
                    &shm,
                    cursor_surface,
                    ThemeSpec::System,
                ) {
                    Ok(pointer) => {
                        self.cx.cursor.pointer = Some(pointer);
                        self.pointer_seat = Some(seat.clone());
                    }
                    Err(e) => tracing::warn!("işaretçi alınamadı: {e}"),
                }
            }
            Capability::Touch if self.touch.is_none() => {
                match self.seat_state.get_touch(qh, &seat) {
                    Ok(touch) => {
                        self.touch = Some(touch);
                        self.touch_seat = Some(seat.clone());
                    }
                    Err(e) => tracing::warn!("dokunmatik alınamadı: {e}"),
                }
            }
            Capability::Keyboard if self.keyboard.is_none() => {
                let handle = self.cx.handle.clone();
                let repeat = Box::new(
                    |state: &mut Self, _: &wl_keyboard::WlKeyboard, event: keyboard::KeyEvent| {
                        state.key_event(event, KeyState::Repeated);
                    },
                );
                match self
                    .seat_state
                    .get_keyboard_with_repeat(qh, &seat, None, handle, repeat)
                {
                    Ok(keyboard) => {
                        self.keyboard = Some(keyboard);
                        self.keyboard_seat = Some(seat.clone());
                        // Pano aygıtları klavyeli koltuğa (seçim odakla ilişkilidir).
                        self.cx.clipboard.seat_added(qh, &seat);
                        // sctk keymap'siz enter'ı yutar; ham odak ayrı bir gözlemciyle izlenir.
                        self.raw_keyboard =
                            Some(seat.get_keyboard(qh, crate::keyfocus::RawKeyboard));
                        // Klavyesi olan koltuk için metin girişi (IME); compositor sunmuyorsa yok.
                        if self.cx.ime.text_input.is_none()
                            && let Some(manager) = &self.cx.ime.manager
                        {
                            self.cx.ime.text_input =
                                Some(manager.get_text_input(&seat, qh, crate::ime::TextInputData));
                        }
                    }
                    Err(e) => tracing::warn!("klavye alınamadı: {e}"),
                }
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer => {
                // ThemedPointer Drop'u shape device'ı, işaretçiyi ve imleç yüzeyini bırakır.
                self.cx.cursor.pointer = None;
                self.pointer_seat = None;
                self.cx.last_press = None;
                self.cx.last_input = None;
            }
            Capability::Touch => {
                if let Some(touch) = self.touch.take() {
                    touch.release();
                }
                self.touch_seat = None;
                self.cx.touch.reset();
                self.cancel_touch_hold();
            }
            Capability::Keyboard => {
                if let Some(keyboard) = self.keyboard.take() {
                    keyboard.release();
                }
                if let Some(keyboard) = self.raw_keyboard.take() {
                    keyboard.release();
                }
                self.focus_gate.reset();
                self.keysyms = None;
                self.cx.vkbd.compositor_keymap = None;
                self.cx.clipboard.seat_removed();
                self.keyboard_seat = None;
                self.cx.last_input = None;
                self.keyboard_focus = None;
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        // sctk koltuğu listeden bu çağrıdan sonra çıkarır.
        let next = self.seat_state.seats().find(|s| *s != seat);
        self.cx.data_control_seat_removed(&seat, next.clone());
        self.cx.idle_seat_removed(&seat, next);
    }
}

impl<A: App> PointerHandler for State<A> {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[pointer::PointerEvent],
    ) {
        for event in events {
            let Some(id) =
                Self::surface_id(&event.surface).filter(|id| self.cx.surfaces.contains_key(id))
            else {
                continue;
            };
            // Kilit varken kilit dışı yüzeye yalnızca Leave geçer (hover durumu temizlensin).
            if !self.cx.input_allowed(id) && !matches!(event.kind, PointerEventKind::Leave { .. }) {
                continue;
            }
            let position = Point::new(event.position.0 as f32, event.position.1 as f32);
            let kind = match &event.kind {
                PointerEventKind::Enter { .. } => {
                    // Compositor enter'da imleci sıfırlar: yüzeyin son şeklini yeniden uygula.
                    let shape = self.cx.cursor.tracker.enter(id);
                    self.cx.cursor.apply(conn, shape);
                    PointerKind::Enter
                }
                PointerEventKind::Leave { .. } => {
                    self.cx.cursor.tracker.leave(id);
                    PointerKind::Leave
                }
                PointerEventKind::Motion { .. } => PointerKind::Motion,
                PointerEventKind::Press { button, serial, .. } => {
                    // begin_move/begin_resize bu serial'ı kullanır.
                    self.cx.last_press = self.pointer_seat.clone().map(|seat| (seat, *serial));
                    self.cx.last_input.clone_from(&self.cx.last_press);
                    PointerKind::Press(Button::from_code(*button))
                }
                PointerEventKind::Release { button, .. } => {
                    PointerKind::Release(Button::from_code(*button))
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    source,
                    ..
                } => {
                    let steps = |a: &pointer::AxisScroll| {
                        if a.value120 != 0 {
                            a.value120 as f32 / 120.0
                        } else {
                            a.discrete as f32
                        }
                    };
                    let scroll = PointerKind::Scroll {
                        delta: Point::new(horizontal.absolute as f32, vertical.absolute as f32),
                        steps: Point::new(steps(horizontal), steps(vertical)),
                    };
                    let moved = horizontal.absolute != 0.0
                        || vertical.absolute != 0.0
                        || horizontal.discrete != 0
                        || vertical.discrete != 0
                        || horizontal.value120 != 0
                        || vertical.value120 != 0;
                    // axis_stop: dokunmatik yüzeyde parmaklar kalktı. Tekerlekte gönderilmez.
                    let stopped = (horizontal.stop || vertical.stop)
                        && *source != Some(wl_pointer::AxisSource::Wheel)
                        && *source != Some(wl_pointer::AxisSource::WheelTilt);
                    match (moved, stopped) {
                        (true, true) => {
                            // Aynı karede son hareket + bitiş: önce hareket, sonra bitiş.
                            self.app.pointer(
                                &mut self.cx,
                                id,
                                PointerEvent::new(position, scroll),
                            );
                            PointerKind::ScrollEnd
                        }
                        (false, true) => PointerKind::ScrollEnd,
                        _ => scroll,
                    }
                }
            };
            self.app
                .pointer(&mut self.cx, id, PointerEvent::new(position, kind));
        }
    }
}

impl<A: App> State<A> {
    /// Dokunma öykünmesinin ürettiği işaretçi olaylarını uygulamaya verir. Yaşamayan yüzeye
    /// gitmez; kilit varken kilit dışı yüzeye yalnızca `Leave` geçer (işaretçiyle aynı kural).
    fn deliver_touch_pointer(&mut self, out: crate::touch::Emitted) {
        for (id, event) in out {
            if !self.cx.surfaces.contains_key(&id)
                || (!self.cx.input_allowed(id) && event.kind != PointerKind::Leave)
            {
                continue;
            }
            self.app.pointer(&mut self.cx, id, event);
        }
    }

    /// Ham dokunma olayı (yaşayan ve girdiye açık yüzeye). `Down` için uygulamanın sahiplenme
    /// kararını döner.
    fn deliver_touch_raw(&mut self, surface: SurfaceId, event: TouchEvent) -> bool {
        if !self.cx.surfaces.contains_key(&surface)
            || (!self.cx.input_allowed(surface) && event.phase != TouchPhase::Cancel)
        {
            return false;
        }
        self.app.touch(&mut self.cx, surface, event)
    }

    fn cancel_touch_hold(&mut self) {
        if let Some(token) = self.cx.touch_hold.take() {
            self.cx.handle.remove(token);
        }
    }

    /// Birincil temas `TOUCH_TAP_DELAY` boyunca basılı kalırsa yerinde bir `Motion`.
    fn arm_touch_hold(&mut self, generation: u64) {
        self.cancel_touch_hold();
        let timer = Timer::from_duration(input::TOUCH_TAP_DELAY);
        let result = self.cx.handle.insert_source(timer, move |_, _, state: &mut Self| {
            state.cx.touch_hold = None;
            let mut out = Vec::new();
            state.cx.touch.hold_tick(generation, &mut out);
            state.deliver_touch_pointer(out);
            TimeoutAction::Drop
        });
        match result {
            Ok(token) => self.cx.touch_hold = Some(token),
            Err(e) => tracing::warn!("dokunma bekleme zamanlayıcısı kurulamadı: {e}"),
        }
    }

    /// Temas başlangıcı (`wl_touch.down` ve sentetik dokunma ortak yolu). Yüzey yaşamıyorsa ya da
    /// girdiye kapalıysa yok sayılır.
    pub(crate) fn touch_down(&mut self, sid: SurfaceId, id: TouchId, position: Point) {
        if !self.cx.surfaces.contains_key(&sid) || !self.cx.input_allowed(sid) {
            return;
        }
        let claimed =
            self.deliver_touch_raw(sid, TouchEvent { id, position, phase: TouchPhase::Down });
        let mut out = Vec::new();
        if let Some(generation) = self.cx.touch.down(sid, id, position, claimed, &mut out) {
            self.arm_touch_hold(generation);
        }
        self.deliver_touch_pointer(out);
    }

    pub(crate) fn touch_up(&mut self, id: TouchId) {
        let mut out = Vec::new();
        if let Some((sid, position)) = self.cx.touch.up(id, &mut out) {
            self.deliver_touch_raw(sid, TouchEvent { id, position, phase: TouchPhase::Up });
        }
        self.deliver_touch_pointer(out);
    }

    /// `position` temasın başladığı yüzeyin koordinatında.
    pub(crate) fn touch_motion(&mut self, id: TouchId, position: Point) {
        let mut out = Vec::new();
        if let Some(sid) = self.cx.touch.motion(id, position, &mut out) {
            self.deliver_touch_raw(sid, TouchEvent { id, position, phase: TouchPhase::Motion });
        }
        self.deliver_touch_pointer(out);
    }

    pub(crate) fn touch_cancel(&mut self) {
        self.cancel_touch_hold();
        let mut out = Vec::new();
        for (id, sid, position) in self.cx.touch.cancel(&mut out) {
            self.deliver_touch_raw(sid, TouchEvent { id, position, phase: TouchPhase::Cancel });
        }
        self.deliver_touch_pointer(out);
    }
}

impl<A: App> TouchHandler for State<A> {
    fn down(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        serial: u32,
        _: u32,
        surface: wl_surface::WlSurface,
        id: i32,
        position: (f64, f64),
    ) {
        let Some(sid) = Self::surface_id(&surface).filter(|id| self.cx.surfaces.contains_key(id))
        else {
            return;
        };
        if !self.cx.input_allowed(sid) {
            return;
        }
        // Dokunma serial'ı da xdg move/resize ve etkinleştirme belirteci için geçerlidir.
        self.cx.last_press = self.touch_seat.clone().map(|seat| (seat, serial));
        self.cx.last_input.clone_from(&self.cx.last_press);
        self.touch_down(sid, TouchId(id), Point::new(position.0 as f32, position.1 as f32));
    }

    fn up(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: u32,
        _: u32,
        id: i32,
    ) {
        self.touch_up(TouchId(id));
    }

    fn motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: u32,
        id: i32,
        position: (f64, f64),
    ) {
        self.touch_motion(TouchId(id), Point::new(position.0 as f32, position.1 as f32));
    }

    fn shape(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: i32,
        _: f64,
        _: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_touch::WlTouch,
        _: i32,
        _: f64,
    ) {
    }

    fn cancel(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_touch::WlTouch) {
        self.touch_cancel();
    }
}

impl<A: App> KeyboardHandler for State<A> {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        // Yalnızca hâlâ yaşayan yüzeyler (yok edilmiş yüzeyin proxy'si bir süre olay alabilir).
        let id = Self::surface_id(surface).filter(|id| self.cx.surfaces.contains_key(id));
        self.keyboard_enter(id);
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        self.keyboard_leave(surface);
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: keyboard::KeyEvent,
    ) {
        self.focus_gate.seen_key(serial);
        self.cx.last_input = self.keyboard_seat.clone().map(|seat| (seat, serial));
        self.key_event(event, KeyState::Pressed);
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: keyboard::KeyEvent,
    ) {
        self.key_event(event, KeyState::Repeated);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: keyboard::KeyEvent,
    ) {
        self.focus_gate.seen_key(serial);
        self.key_event(event, KeyState::Released);
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: keyboard::Modifiers,
        _: RawModifiers,
        layout: u32,
    ) {
        let new = Modifiers {
            ctrl: modifiers.ctrl,
            shift: modifiers.shift,
            alt: modifiers.alt,
            logo: modifiers.logo,
            caps_lock: modifiers.caps_lock,
            num_lock: modifiers.num_lock,
        };
        let changed = new != self.modifiers;
        self.modifiers = new;
        // Odak varsa bildir (ör. kilit ekranının "Caps Lock açık" uyarısı); her KeyEvent de
        // güncel değerleri taşır.
        if changed
            && let Some(id) = self
                .keyboard_focus
                .filter(|id| self.cx.surfaces.contains_key(id) && self.cx.input_allowed(*id))
        {
            self.app.modifiers_changed(&mut self.cx, id, new);
        }
        if let Some(r) = self.keysyms.as_mut() {
            r.set_layout(layout);
        }
        let layout = layout as usize;
        if layout != self.cx.ime.active_layout {
            self.cx.ime.active_layout = layout;
            tracing::debug!(layout = self.cx.ime.layout_name(), "klavye düzeni değişti");
            self.app.keyboard_layout_changed(&mut self.cx);
        }
    }

    fn update_keymap(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        keymap: keyboard::Keymap<'_>,
    ) {
        self.focus_gate.has_keymap = true;
        self.reconcile_keyboard_focus();
        let text = keymap.as_string();
        self.keysyms = crate::keysym::KeysymResolver::from_keymap_text(&text);
        if self.keysyms.is_none() {
            tracing::warn!("keymap ikinci kez derlenemedi; tuş olayları XKB ayrıntısı taşımayacak");
        }
        let layouts = crate::ime::parse_layout_names(&text);
        self.cx.vkbd.compositor_keymap = Some(text);
        if layouts != self.cx.ime.layouts {
            tracing::debug!(?layouts, "klavye düzenleri");
            self.cx.ime.layouts = layouts;
            self.app.keyboard_layout_changed(&mut self.cx);
        }
    }
}

impl<A: App> ProvidesRegistryState for State<A> {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![
        OutputState,
        SeatState,
        crate::focus_grab::FocusGrabGlobal,
        crate::color_transform::ColorTransformGlobal,
        crate::idle::IdleNotifyGlobal,
        crate::virtual_keyboard::VirtualKeyboardGlobal,
        crate::data_control::DataControlGlobal,
        crate::toplevels::ToplevelsGlobal,
        crate::capture::CaptureGlobal
    ];
}

/// Olayı olmayan protokol nesneleri (viewporter, viewport, fractional scale yöneticisi).
#[derive(Debug)]
pub(crate) struct NoEvents;

/// `wl_shm.format` olayları kullanılmaz (imleç teması kendi biçimini seçer).
impl<D> Dispatch2<wayland_client::protocol::wl_shm::WlShm, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &wayland_client::protocol::wl_shm::WlShm,
        _: wayland_client::protocol::wl_shm::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

impl<D> Dispatch2<WpViewporter, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &WpViewporter,
        _: <WpViewporter as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

impl<D> Dispatch2<WpViewport, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &WpViewport,
        _: <WpViewport as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

impl<D> Dispatch2<WpFractionalScaleManagerV1, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &WpFractionalScaleManagerV1,
        _: <WpFractionalScaleManagerV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

/// `wp_fractional_scale_v1` kullanıcı verisi: hangi yüzeyin ölçeği.
#[derive(Debug)]
pub(crate) struct FractionalData(pub SurfaceId);

impl<A: App> Dispatch2<WpFractionalScaleV1, State<A>> for FractionalData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            // Protokol: ölçek = scale / 120.
            state.set_scale(self.0, (scale as f32 / 120.0).max(0.25));
        }
    }
}

delegate_registry!(@<A: App> State<A>);
delegate_dispatch2!(@<A: App> State<A>);

impl<A: App> State<A> {
    /// Klavye odağı `id`'ye geçti (sctk'nin enter'ı ya da keymap sonrası yeniden uygulama).
    fn keyboard_enter(&mut self, id: Option<SurfaceId>) {
        if let Some(old) = self.keyboard_focus.take()
            && Some(old) != id
            && self.cx.surfaces.contains_key(&old)
        {
            #[cfg(feature = "a11y")]
            self.cx.a11y.focus_changed(old, false, false);
            // Protokol enter'dan önce leave gönderir; yine de tutarlı kal.
            self.app.keyboard_focus(&mut self.cx, old, false);
        }
        self.keyboard_focus = id;
        if let Some(id) = id.filter(|id| self.cx.input_allowed(*id)) {
            #[cfg(feature = "a11y")]
            self.cx.a11y.focus_changed(id, true, self.cx.input_allowed(id));
            self.app.keyboard_focus(&mut self.cx, id, true);
        }
    }

    fn keyboard_leave(&mut self, surface: &wl_surface::WlSurface) {
        // Basılı tuşun tekrarını sctk leave'de durdurur (current_repeat temizlenir).
        let id = Self::surface_id(surface);
        if self.keyboard_focus.is_some() && self.keyboard_focus == id {
            self.keyboard_focus = None;
        }
        if let Some(id) = id.filter(|id| self.cx.surfaces.contains_key(id)) {
            #[cfg(feature = "a11y")]
            self.cx.a11y.focus_changed(id, false, false);
            self.app.keyboard_focus(&mut self.cx, id, false);
        }
    }

    /// Gözlemci klavyenin enter'ı (keymap olsun olmasın gelir).
    pub(crate) fn raw_keyboard_enter(&mut self, surface: &wl_surface::WlSurface) {
        let id = Self::surface_id(surface).filter(|id| self.cx.surfaces.contains_key(id));
        self.focus_gate.raw_enter(id);
    }

    /// Keymap varken sctk'nin yuttuğu odağı uygular. `update_keymap`'te ve tur sonunda.
    fn reconcile_keyboard_focus(&mut self) {
        let surfaces = &self.cx.surfaces;
        if let Some(id) = self
            .focus_gate
            .focus_to_replay(self.keyboard_focus, |id| surfaces.contains_key(&id))
        {
            tracing::debug!(?id, "keymap sonrası klavye odağı yeniden uygulandı");
            self.keyboard_enter(Some(id));
        }
    }

    /// Tur sonunda: odak uzlaştırması, sonra keymap'siz gelip sctk'nin yuttuğu tuşların
    /// (yedek keymap kurulduktan sonra) yeniden oynatılması. Tuşlar yalnızca basıldıkları yüzey
    /// hâlâ (ham ve teslim edilen) odaktaysa ve girdi kabul ediyorsa oynatılır; gerisi atılır.
    fn reconcile_keyboard(&mut self) {
        self.reconcile_keyboard_focus();
        let cx = &self.cx;
        let keys = self.focus_gate.take_swallowed(self.keyboard_focus, |id| {
            cx.surfaces.contains_key(&id) && cx.input_allowed(id)
        });
        if keys.is_empty() {
            return;
        }
        let Some(keyboard) = self.keyboard.clone() else {
            return;
        };
        let (conn, qh) = (self.cx.conn.clone(), self.cx.qh.clone());
        tracing::debug!(count = keys.len(), "keymap'siz tuşlar yeniden oynatılıyor");
        for key in keys.iter().copied() {
            crate::keyfocus::feed_sctk(
                self,
                &keyboard,
                crate::keyfocus::key_event(key),
                &conn,
                &qh,
            );
        }
    }

    /// Compositor keymap göndermeden tuş geldi: sctk'ye yerleşik yedek keymap verilir (bir kez).
    pub(crate) fn install_fallback_keymap(&mut self, conn: &Connection, qh: &QueueHandle<Self>) {
        let Some(keyboard) = self.keyboard.clone() else {
            return;
        };
        let Some((fd, size)) = crate::keyfocus::fallback_keymap() else {
            tracing::error!("compositor klavye eşlemi göndermedi ve yedek eşlem derlenemedi");
            return;
        };
        tracing::warn!(
            layout = crate::keyfocus::FALLBACK_LAYOUT,
            "compositor klavye eşlemi göndermedi; yedek düzen kullanılıyor"
        );
        crate::keyfocus::feed_sctk(
            self,
            &keyboard,
            crate::keyfocus::keymap_event(fd, size),
            conn,
            qh,
        );
    }
}

// Erişilebilirlik (ADR-0011, `a11y` özelliği).
#[cfg(feature = "a11y")]
impl<A: App> State<A> {
    /// Bu yüzeyin ağacı/eylemi AT'ye açık mı: uygulama askıya almadı ve girdisi açık (kilitliyken
    /// yalnız kilit yüzeyleri; onlara adaptör kurulmaz). Kapalıysa ağaç içeriksiz, eylem atılır.
    fn a11y_allowed(&self, surface: SurfaceId) -> bool {
        !self.cx.a11y.is_suspended() && self.cx.input_allowed(surface)
    }

    /// Adaptör thread'inden gelen olay (calloop kanalı). Eylem yalnız izinli yüzeye iletilir:
    /// askıdayken (ör. oturum kilitli) ya da girdisi kapalı yüzeyde a11y bus'ındaki herhangi bir
    /// sürecin kabuğun düğmelerine bastırması engellenir.
    pub(crate) fn a11y_event(&mut self, event: crate::a11y::A11yEvent) {
        if let Some((surface, request)) = self.cx.a11y.event(event)
            && self.cx.surfaces.contains_key(&surface)
        {
            if !self.a11y_allowed(surface) {
                tracing::info!(?surface, action = ?request.action, "erişilebilirlik: askıda/girdisi kapalı yüzeye AT eylemi reddedildi");
                return;
            }
            self.app.accessibility_action(&mut self.cx, surface, request);
        }
    }

    /// Olay turunun sonunda, çizimlerden sonra: etkin yüzeylerin ağacını gerekiyorsa kurar.
    /// Bu turda çizilen yüzeyler `draw_queue`'dadır (`after_dispatch`). İzinsiz yüzeyin ağacı
    /// içeriksizleştirilir; uygulamaya hiç sorulmaz.
    pub(crate) fn a11y_update(&mut self) {
        let alive = &self.cx.surfaces;
        self.cx.a11y.retain(|id| alive.contains_key(&id));
        let now = Instant::now();
        let ids: Vec<SurfaceId> = self.cx.surfaces.keys().copied().collect();
        for id in ids {
            if !self.a11y_allowed(id) {
                if self.cx.a11y.needs_blank(id) {
                    self.cx.a11y.blank(id, now);
                }
                continue;
            }
            let Some(s) = self.cx.surfaces.get(&id) else { continue };
            let due = self.cx.a11y.due(id, self.draw_queue.contains(&id), s.wants_frame, s.has_buffer, now)
                || (self.cx.a11y.is_blank(id) && self.cx.a11y.is_active(id) && s.has_buffer);
            if !due {
                continue;
            }
            let mut tree = nuxut_access::AccessTree::new();
            self.app.accessibility(&mut self.cx, id, &mut tree);
            if tree.duplicates() > 0 {
                tracing::warn!(?id, duplicates = tree.duplicates(), "erişilebilirlik: yinelenen düğüm kimlikleri atıldı");
            }
            let bounds = self.a11y_bounds(id);
            self.cx.a11y.apply(id, tree.finish(), bounds, now);
        }
    }

    /// Yüzeyin ekran dikdörtgeni: layer yüzeyinde çıkış + anchor + margin, pencerede (0,0)
    /// (Wayland istemciye pencere konumunu vermez; ADR-0011 bilinen sınır).
    fn a11y_bounds(&self, id: SurfaceId) -> nuxut_core::Rect {
        let Some(entry) = self.cx.surfaces.get(&id) else {
            return nuxut_core::Rect::ZERO;
        };
        let size = (entry.size.0 as f32, entry.size.1 as f32);
        let output = entry
            .output
            .and_then(|o| self.cx.outputs.get(&o))
            .and_then(|o| self.output_info(&o.wl))
            .map(|i| nuxut_core::Rect::new(i.position.x, i.position.y, i.size.width, i.size.height));
        match (self.cx.a11y.place(id), output) {
            (Some(place), Some(out)) => crate::a11y::layer_rect(out, place, size),
            _ => nuxut_core::Rect::new(0.0, 0.0, size.0, size.1),
        }
    }
}

/// Kip boyutunu (panelin kendi yönü) compositor uzayının yönüne çevirir: 90/270 derece
/// dönüşlerde genişlik ve yükseklik yer değiştirir; yansıtma eksenleri değiştirmez.
fn oriented_mode_size(mode: (u32, u32), transform: wl_output::Transform) -> (u32, u32) {
    use wl_output::Transform as T;
    match transform {
        T::_90 | T::_270 | T::Flipped90 | T::Flipped270 => (mode.1, mode.0),
        _ => mode,
    }
}

#[cfg(test)]
mod output_tests {
    use super::{oriented_mode_size, wl_output::Transform};

    #[test]
    fn mode_size_follows_transform() {
        let m = (2560, 1440);
        for t in [Transform::Normal, Transform::_180, Transform::Flipped, Transform::Flipped180] {
            assert_eq!(oriented_mode_size(m, t), m);
        }
        for t in [Transform::_90, Transform::_270, Transform::Flipped90, Transform::Flipped270] {
            assert_eq!(oriented_mode_size(m, t), (1440, 2560));
        }
    }
}

#[cfg(test)]
mod tests {
    use khronos_egl as egl;
    use nuxut_core::{Color, Corners, Rect};
    use nuxut_render::glow::{self, HasContext};
    use nuxut_render::{RectStyle, Renderer, TextSystem};

    const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

    /// Başsız GLES 3 context'i (surfaceless EGL); yoksa `None` (test atlanır).
    fn headless() -> Option<(
        egl::DynamicInstance<egl::EGL1_5>,
        egl::Display,
        glow::Context,
    )> {
        // SAFETY: test; libEGL sistem kütüphanesi, API sözleşmesine uyar.
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }.ok()?;
        // SAFETY: surfaceless platformu yerel ekran istemez (NULL).
        let display = unsafe {
            egl.get_platform_display(
                PLATFORM_SURFACELESS_MESA,
                egl::DEFAULT_DISPLAY,
                &[egl::ATTRIB_NONE],
            )
        }
        .ok()?;
        egl.initialize(display).ok()?;
        egl.bind_api(egl::OPENGL_ES_API).ok()?;
        let config = egl
            .choose_first_config(
                display,
                &[
                    egl::SURFACE_TYPE,
                    0,
                    egl::RENDERABLE_TYPE,
                    egl::OPENGL_ES3_BIT,
                    egl::NONE,
                ],
            )
            .ok()??;
        let context = egl
            .create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE],
            )
            .ok()?;
        egl.make_current(display, None, None, Some(context)).ok()?;
        // SAFETY: context güncel; işlev işaretçileri bu context için yüklenir.
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                egl.get_proc_address(name)
                    .map_or(std::ptr::null(), |f| f as *const _)
            })
        };
        Some((egl, display, gl))
    }

    #[test]
    fn lock_frame_is_opaque_even_if_app_paints_translucent() {
        let Some((egl, display, gl)) = headless() else {
            eprintln!("EGL surfaceless yok; test atlandı");
            return;
        };
        let (w, h) = (64u32, 32u32);
        let mut renderer = Renderer::new(gl).expect("shader'lar derlenmeli");
        // SAFETY: context güncel; FBO ve doku bu testte yaratılır.
        unsafe {
            let gl = renderer.gl();
            let tex = gl.create_texture().unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
            let fbo = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
        }
        let mut text = TextSystem::new();
        // Sol yarı yarı saydam kırmızı, sağ yarı hiç çizilmez (renderer saydam temizler).
        renderer.render(&mut text, (w, h), 1.0, |p| {
            p.rect(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                &RectStyle::filled(Color::rgba(1.0, 0.0, 0.0, 0.5), Corners::ZERO),
            );
        });
        super::force_opaque(renderer.gl());
        let mut px = vec![0u8; (w * h * 4) as usize];
        // SAFETY: context güncel; tampon w*h*4 bayt.
        unsafe {
            renderer.gl().read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut px)),
            );
        }
        assert!(px.chunks(4).all(|p| p[3] == 255), "tüm alfa 255 olmalı");
        let at = |x: u32, y: u32| &px[((y * w + x) * 4) as usize..][..4];
        // Önceden çarpılmış kırmızı (≈128) korunur; çizilmeyen yer opak siyah.
        assert!((120..=136).contains(&at(8, 16)[0]), "{:?}", at(8, 16));
        assert_eq!(at(48, 16), [0, 0, 0, 255]);
        drop(renderer);
        let _ = egl.make_current(display, None, None, None);
        let _ = egl.terminate(display);
    }
}
