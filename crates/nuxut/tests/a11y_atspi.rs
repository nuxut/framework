//! Uçtan uca ekran okuyucu testi (ADR-0011): gerçek runtime + gerçek AT-SPI.
//!
//! Bir layer yüzeyinde birkaç bileşen çizen uygulama başlatılır; test bir yardımcı teknoloji
//! gibi davranır: oturum bus'ında `org.a11y.Status.IsEnabled`'ı açar, a11y bus'ına bağlanır,
//! registry'den uygulamamızı bulur, ağacı (rol, ad, durum, değer, metin) okur, bir düğmeye
//! `DoAction` gönderir ve metin yazma girişiminin (`SetTextContents`) uygulamaya ULAŞMADIĞINI
//! doğrular.
//!
//! Yalnız yalıtılmış ortamda koşar: `tools/a11y-e2e.sh` (headless sway test compositor'ı +
//! `dbus-run-session` + at-spi-bus-launcher + at-spi2-registryd; kullanıcının oturum ve a11y
//! bus'ına dokunmaz). `NUXUT_A11Y_E2E=1` yoksa ya da `a11y` özelliği kapalıysa nedenini yazıp
//! atlanır (`cargo test --workspace`'te böyle).

#[cfg(all(target_os = "linux", feature = "a11y"))]
mod e2e {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use atspi::proxy::accessible::AccessibleProxy;
    use atspi::proxy::action::ActionProxy;
    use atspi::proxy::editable_text::EditableTextProxy;
    use atspi::proxy::text::TextProxy;
    use atspi::proxy::value::ValueProxy;
    use atspi::{Role, State};
    use nuxut::access::{AccessAction, AccessRequest, AccessTree, root_child};
    use nuxut::ui::{Button, ButtonVariant, Orientation, Slider, TextInput, Toggle, Ui};
    use nuxut::{
        Anchor, App, Context, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, OutputInfo, Painter, PointerEvent, Rect, SurfaceId,
        TimerId,
    };

    /// Testten uygulamaya: erişilebilirliği askıya al (kilit benzetimi; zamanlayıcıda okunur).
    static SUSPEND: AtomicBool = AtomicBool::new(false);

    /// Uygulamadan teste: alınan AT eylemleri.
    type Seen = mpsc::Sender<AccessRequest>;

    struct Demo {
        surface: Option<SurfaceId>,
        /// Ek yüzey (ana yüzeyden önce yaratılır, geç çizilir): çok adaptörlü etkinleşme sırası.
        extra: Option<SurfaceId>,
        extra_drawn: bool,
        save: Button,
        wifi: Toggle,
        volume: Slider,
        search: TextInput,
        password: TextInput,
        seen: Seen,
        quit_after: Instant,
        tick: Option<TimerId>,
    }

    impl Demo {
        fn new(seen: Seen) -> Self {
            let theme = nuxut::Theme::default();
            let mut search = TextInput::new(&theme).with_placeholder("Ara");
            search.set_text("fire");
            search.set_focused(true, Instant::now());
            let mut password = TextInput::new(&theme).password(true);
            password.set_text("hunter2");
            Self {
                surface: None,
                extra: None,
                extra_drawn: false,
                save: Button::new(&theme, ButtonVariant::Filled).with_label(&theme, "Kaydet"),
                wifi: Toggle::new(&theme, true),
                volume: Slider::new(&theme, Orientation::Horizontal, 0.25).with_steps(20),
                search,
                password,
                seen,
                quit_after: Instant::now() + Duration::from_secs(60),
                tick: None,
            }
        }
    }

    impl App for Demo {
        type Message = ();

        fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
            if self.surface.is_some() {
                return;
            }
            let spec = LayerSurfaceSpec {
                layer: Layer::Top,
                anchor: Anchor::TOP | Anchor::LEFT,
                size: (360, 320),
                keyboard: KeyboardMode::OnDemand,
                ..LayerSurfaceSpec::new("nuxut-a11y-test", output.id)
            };
            let extra = LayerSurfaceSpec { anchor: Anchor::BOTTOM | Anchor::RIGHT, size: (40, 40), keyboard: KeyboardMode::None, ..spec.clone() };
            self.extra = Some(cx.create_layer_surface(extra));
            self.surface = Some(cx.create_layer_surface(spec));
            self.tick = Some(cx.set_timer(Duration::from_millis(200)));
        }

        fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

        fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

        fn timer(&mut self, cx: &mut Context<Self>, _id: TimerId) {
            let suspend = SUSPEND.load(Ordering::SeqCst);
            if suspend != cx.a11y_suspended() {
                cx.set_a11y_suspended(suspend);
                if let Some(s) = self.surface {
                    cx.request_frame(s);
                }
            }
            // Test bitince (kanal kapandı) ya da güvenlik süresinde çık.
            if self.seen.send(AccessRequest { node: root_child("ping"), action: AccessAction::Focus }).is_err()
                || Instant::now() > self.quit_after
            {
                cx.quit();
            } else {
                self.tick = Some(cx.set_timer(Duration::from_millis(200)));
            }
        }

        fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
            if Some(surface) == self.extra {
                // İlk karede çizim ertelenir: ana yüzey önce etkinleşip ağaç alsın.
                if !self.extra_drawn {
                    self.extra_drawn = true;
                    cx.request_frame(surface);
                }
                return;
            }
            let theme = cx.theme().clone();
            self.save.set_rect(Rect::new(16.0, 16.0, 120.0, 40.0));
            self.wifi.set_rect(Rect::new(16.0, 72.0, 52.0, 32.0));
            self.volume.set_rect(Rect::new(16.0, 120.0, 300.0, 44.0));
            self.search.set_rect(Rect::new(16.0, 176.0, 300.0, 48.0));
            self.password.set_rect(Rect::new(16.0, 240.0, 300.0, 48.0));
            let mut ui = Ui::new(painter, &theme, now);
            self.save.draw(&mut ui);
            self.wifi.draw(&mut ui);
            self.volume.draw(&mut ui);
            self.search.draw(&mut ui);
            self.password.draw(&mut ui);
        }

        fn accessibility(&mut self, _cx: &mut Context<Self>, surface: SurfaceId, tree: &mut AccessTree) {
            if Some(surface) == self.extra {
                tree.root().label("Ek yüzey");
                return;
            }
            tree.root().label("Nuxut test");
            self.save.access(tree, "save");
            self.wifi.access(tree, "wifi").label("Wi-Fi");
            self.volume.access(tree, "volume", 0.0..=100.0).label("Ses");
            self.search.access(tree, "search").label("Uygulama ara");
            self.password.access(tree, "password").label("Parola");
        }

        fn accessibility_action(&mut self, cx: &mut Context<Self>, surface: SurfaceId, request: AccessRequest) {
            if request.node == root_child("wifi") {
                let _ = self.wifi.access_action(request.action, Instant::now());
            }
            let _ = self.seen.send(request);
            cx.request_frame(surface);
        }
    }

    struct Node {
        name: String,
        role: Role,
        dest: String,
        path: String,
    }

    async fn walk(conn: &zbus::Connection, dest: &str, path: &str, out: &mut Vec<Node>, depth: usize) -> zbus::Result<()> {
        let acc = AccessibleProxy::builder(conn).destination(dest.to_owned())?.path(path.to_owned())?.cache_properties(zbus::proxy::CacheProperties::No).build().await?;
        out.push(Node { name: acc.name().await?, role: acc.get_role().await?, dest: dest.to_owned(), path: path.to_owned() });
        if depth > 8 {
            return Ok(());
        }
        for child in acc.get_children().await? {
            let d = child.name_as_str().unwrap_or(dest).to_owned();
            Box::pin(walk(conn, &d, child.path_as_str(), out, depth + 1)).await?;
        }
        Ok(())
    }

    async fn a11y_bus() -> zbus::Result<(zbus::Connection, zbus::Connection)> {
        let session = zbus::Connection::session().await?;
        // Bir AT gibi: erişilebilirliği aç (yalıtılmış bus; kullanıcının ayarı değil).
        session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "Set",
                &("org.a11y.Status", "IsEnabled", zbus::zvariant::Value::from(true)),
            )
            .await?;
        let reply = session.call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &()).await?;
        let address: String = reply.body().deserialize()?;
        let a11y = zbus::connection::Builder::address(address.as_str())?.build().await?;
        Ok((session, a11y))
    }

    pub fn run() {
        let (tx, rx) = mpsc::channel();
        let app = std::thread::spawn(move || nuxut::run(Demo::new(tx)));

        zbus::block_on(async {
            let (_session, bus) = a11y_bus().await.expect("a11y bus'ına bağlanılamadı");
            let exe = std::env::current_exe().unwrap();
            let app_name = exe.file_name().unwrap().to_string_lossy().into_owned();

            // Uygulama registry'de belirene ve ağaç dolana dek bekle (IsEnabled → etkinleşme →
            // ilk çizim → ağaç).
            let deadline = Instant::now() + Duration::from_secs(20);
            let nodes = loop {
                let mut nodes = Vec::new();
                let walked = walk(&bus, "org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root", &mut nodes, 0).await;
                let ours = nodes.iter().any(|n| n.name == app_name);
                if ours && nodes.iter().any(|n| n.name == "Parola") {
                    break nodes;
                }
                assert!(Instant::now() < deadline, "uygulama ağacı AT-SPI'de görünmedi ({} düğüm, {:?})", nodes.len(), walked.err());
                async_io::Timer::after(Duration::from_millis(100)).await;
            };
            let find = |name: &str| nodes.iter().find(|n| n.name == name).unwrap_or_else(|| panic!("AT-SPI'de yok: {name}"));

            let frame = find("Nuxut test");
            assert!(matches!(frame.role, Role::Frame | Role::Window), "kök rolü: {:?}", frame.role);
            assert_eq!(find("Kaydet").role, Role::Button);
            let wifi = find("Wi-Fi");
            assert_eq!(wifi.role, Role::ToggleButton, "anahtar rolü");
            assert_eq!(find("Ses").role, Role::Slider);
            assert_eq!(find("Uygulama ara").role, Role::Entry);
            assert_eq!(find("Parola").role, Role::PasswordText);
            assert!(!nodes.iter().any(|n| n.name.contains("hunter")), "parola ağaçta");

            let proxy = |n: &Node| {
                let (d, p) = (n.dest.clone(), n.path.clone());
                let bus = bus.clone();
                async move { AccessibleProxy::builder(&bus).destination(d).unwrap().path(p).unwrap().cache_properties(zbus::proxy::CacheProperties::No).build().await.unwrap() }
            };
            let state = |n: &Node| {
                let fut = proxy(n);
                async move { fut.await.get_state().await.unwrap() }
            };
            let s = state(wifi).await;
            assert!(s.contains(State::Pressed), "Wi-Fi açık okunmalı: {s:?}");
            assert!(state(find("Uygulama ara")).await.contains(State::Focused), "odak arama alanında");

            let ses = find("Ses");
            let value = ValueProxy::builder(&bus).destination(ses.dest.clone()).unwrap().path(ses.path.clone()).unwrap().cache_properties(zbus::proxy::CacheProperties::No).build().await.unwrap();
            assert_eq!(value.current_value().await.unwrap(), 25.0);

            let search = find("Uygulama ara");
            let text = TextProxy::builder(&bus).destination(search.dest.clone()).unwrap().path(search.path.clone()).unwrap().cache_properties(zbus::proxy::CacheProperties::No).build().await.unwrap();
            assert_eq!(text.get_text(0, -1).await.unwrap(), "fire");
            let pw = find("Parola");
            let pw_text = TextProxy::builder(&bus).destination(pw.dest.clone()).unwrap().path(pw.path.clone()).unwrap().cache_properties(zbus::proxy::CacheProperties::No).build().await.unwrap();
            assert_eq!(pw_text.get_text(0, -1).await.unwrap(), "•••••••");

            // Metin yazma girişimi uygulamaya ulaşmamalı (ADR-0011 karar 6).
            let edit = EditableTextProxy::builder(&bus).destination(pw.dest.clone()).unwrap().path(pw.path.clone()).unwrap().cache_properties(zbus::proxy::CacheProperties::No).build().await.unwrap();
            let _ = edit.set_text_contents("injected").await;

            // Anahtarı AT'den çevir: uygulama Click alır, durum ağaca yansır.
            let action = ActionProxy::builder(&bus).destination(wifi.dest.clone()).unwrap().path(wifi.path.clone()).unwrap().cache_properties(zbus::proxy::CacheProperties::No).build().await.unwrap();
            assert!(action.do_action(0).await.unwrap());
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut got = Vec::new();
            while Instant::now() < deadline {
                while let Ok(r) = rx.try_recv() {
                    if r.node != root_child("ping") {
                        got.push(r);
                    }
                }
                if got.iter().any(|r| r.action == AccessAction::Click) && !state(wifi).await.contains(State::Pressed) {
                    break;
                }
                async_io::Timer::after(Duration::from_millis(100)).await;
            }
            assert_eq!(got, vec![AccessRequest { node: root_child("wifi"), action: AccessAction::Click }], "yalnız tıklama gelmeli");
            assert!(!state(wifi).await.contains(State::Pressed), "anahtar kapandı okunmalı");

            // --- Düşmanca: askıdayken (ör. oturum kilitli) ağaç içeriksiz, eylem reddedilir. ---
            SUSPEND.store(true, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let mut now_nodes = Vec::new();
                let _ = walk(&bus, "org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root", &mut now_nodes, 0).await;
                let ours = now_nodes.iter().any(|n| n.name == app_name);
                if ours && !now_nodes.iter().any(|n| ["Kaydet", "Wi-Fi", "Parola", "Nuxut test"].contains(&n.name.as_str())) {
                    break;
                }
                assert!(Instant::now() < deadline, "askıdayken ağaç içeriksizleşmedi");
                async_io::Timer::after(Duration::from_millis(100)).await;
            }
            // Eski düğüm yollarıyla eylem: uygulamaya ulaşmamalı.
            let _ = action.do_action(0).await;
            let save = find("Kaydet");
            if let Ok(a) = ActionProxy::builder(&bus).destination(save.dest.clone()).unwrap().path(save.path.clone()).unwrap().cache_properties(zbus::proxy::CacheProperties::No).build().await {
                let _ = a.do_action(0).await;
            }
            async_io::Timer::after(Duration::from_millis(800)).await;
            let leaked: Vec<_> = rx.try_iter().filter(|r| r.node != root_child("ping")).collect();
            assert!(leaked.is_empty(), "askıdayken AT eylemi uygulamaya ulaştı: {leaked:?}");

            // Sürdürünce ağaç geri gelir.
            SUSPEND.store(false, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let mut now_nodes = Vec::new();
                let _ = walk(&bus, "org.a11y.atspi.Registry", "/org/a11y/atspi/accessible/root", &mut now_nodes, 0).await;
                if now_nodes.iter().any(|n| n.name == "Kaydet") {
                    break;
                }
                assert!(Instant::now() < deadline, "sürdürünce ağaç geri gelmedi");
                async_io::Timer::after(Duration::from_millis(100)).await;
            }
        });
        drop(rx);
        app.join().expect("uygulama thread'i").expect("runtime hatası");
    }
}

#[test]
fn screen_reader_tree_over_atspi() {
    if std::env::var_os("NUXUT_A11Y_E2E").is_none() {
        eprintln!("atlandı: NUXUT_A11Y_E2E yok — yalıtılmış ortam gerekir (tools/a11y-e2e.sh)");
        return;
    }
    #[cfg(not(all(target_os = "linux", feature = "a11y")))]
    panic!("NUXUT_A11Y_E2E verildi ama `a11y` özelliği kapalı: cargo test -p nuxut --features a11y --test a11y_atspi");
    #[cfg(all(target_os = "linux", feature = "a11y"))]
    e2e::run();
}
