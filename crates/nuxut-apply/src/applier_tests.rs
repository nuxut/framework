//! Applier testleri: hepsi geçici XDG kökleri altında, test örneği ortamıyla (kullanıcının
//! dosyalarına, dconf'una ve systemd'sine dokunulmaz).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::*;
use crate::model::{
    KeyPath, MAX_ADDRESS_BYTES, MAX_FRAGMENTS, MAX_LABEL_CHARS, MAX_PATH_BYTES, Position,
};

/// Alt süreç kapısı (bkz. `crate::test_gate`): fixture yaşadıkça tutulur.
enum Gate {
    Shared(#[allow(dead_code)] std::sync::RwLockReadGuard<'static, ()>),
    Exclusive(#[allow(dead_code)] std::sync::RwLockWriteGuard<'static, ()>),
}

struct Fx {
    _t: tempfile::TempDir,
    root: PathBuf,
    clock: Arc<AtomicU64>,
    app: Applier,
    _gate: Gate,
}

impl Fx {
    /// Depo/kilit kullanan test: alt süreç başlatan testlerle örtüşmez.
    fn new() -> Self {
        Self::with_gate(Gate::Shared(crate::test_gate::shared()))
    }

    /// Hem depo kullanan hem alt süreç başlatan test (ör. gerçek arka uç yolu).
    fn exclusive() -> Self {
        Self::with_gate(Gate::Exclusive(crate::test_gate::exclusive()))
    }

    fn with_gate(gate: Gate) -> Self {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_path_buf();
        fs::create_dir_all(root.join("cfg")).unwrap();
        let env = Env {
            home: Some(root.join("home")),
            real_home: crate::env::real_home(),
            config_home: Some(root.join("cfg")),
            state_home: Some(root.join("state")),
            test_instance: true,
            hyprland: None,
            disabled: false,
        };
        // 2026-10-10 12:00:00 UTC'den başlayan elle ilerletilen saat.
        let clock = Arc::new(AtomicU64::new(1_791_633_600_000));
        let mut app = Applier::open(env);
        let c = clock.clone();
        app.set_clock(Box::new(move || {
            UNIX_EPOCH + Duration::from_millis(c.load(Ordering::SeqCst))
        }));
        app.set_lock_wait(Duration::from_millis(100));
        Self {
            _t: t,
            root,
            clock,
            app,
            _gate: gate,
        }
    }

    fn advance(&self, ms: u64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn cfg(&self, rel: &str) -> PathBuf {
        self.root.join("cfg").join(rel)
    }

    fn snapshots(&self) -> PathBuf {
        self.root.join("state/nuxut/snapshots")
    }

    fn snap_count(&self) -> usize {
        self.app
            .list(&Filter {
                setting: None,
                limit: 200,
            })
            .unwrap()
            .len()
    }
}

fn sid(s: &str) -> SettingId {
    SettingId::new(s).unwrap()
}

fn ini(file: &Path, section: &str, key: &str) -> Fragment {
    Fragment::IniKey {
        file: file.to_path_buf(),
        section: section.into(),
        key: key.into(),
    }
}

fn text(s: &str) -> FragValue {
    FragValue::Text(s.into())
}

fn change(setting: &str, writes: Vec<(Fragment, FragValue)>) -> Change {
    Change {
        setting: sid(setting),
        label: "test".into(),
        source: Source::Settings,
        writes,
        create_missing: Vec::new(),
        take_over: Vec::new(),
    }
}

fn applied(o: Outcome) -> SnapshotId {
    match o {
        Outcome::Applied { id } => id,
        other => panic!("beklenen applied, gelen {other:?}"),
    }
}

const GTK: &str =
    "# kullanıcının yorumu\n[Settings]\ngtk-theme-name = Adwaita-dark\ngtk-font-name=Yazi 11\n";

#[test]
fn apply_and_restore_bytewise() {
    let fx = Fx::new();
    let f = fx.cfg("gtk-3.0/settings.ini");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(&f, GTK).unwrap();
    fs::set_permissions(&f, fs::Permissions::from_mode(0o640)).unwrap();

    let c = change(
        "look.gtk",
        vec![
            (ini(&f, "Settings", "gtk-theme-name"), text("Nuxut")),
            (ini(&f, "Settings", "gtk-cursor-theme-name"), text("Bibata")),
        ],
    );
    let id = applied(fx.app.apply(c.clone()));
    let now = fs::read_to_string(&f).unwrap();
    assert!(
        now.contains("gtk-theme-name = Nuxut\n") && now.contains("gtk-cursor-theme-name=Bibata\n"),
        "{now}"
    );
    assert!(now.starts_with("# kullanıcının yorumu\n"));
    assert_eq!(
        fs::metadata(&f).unwrap().permissions().mode() & 0o777,
        0o640
    );
    // Aynısı: noop, yeni snapshot yok.
    fx.advance(10_000);
    assert_eq!(fx.app.apply(c), Outcome::Noop);
    assert_eq!(fx.snap_count(), 1);

    // Depo izinleri.
    assert_eq!(
        fs::metadata(fx.snapshots()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let sdir = fx.snapshots().join(id.as_str());
    assert_eq!(
        fs::metadata(&sdir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for e in fs::read_dir(&sdir).unwrap() {
        let e = e.unwrap();
        assert_eq!(
            e.metadata().unwrap().permissions().mode() & 0o777,
            0o600,
            "{:?}",
            e.file_name()
        );
    }
    // Manifest değer içermez (blob'larda).
    let manifest = fs::read_to_string(sdir.join("manifest.toml")).unwrap();
    assert!(
        !manifest.contains("Nuxut")
            && !manifest.contains("Adwaita")
            && !manifest.contains("Bibata"),
        "{manifest}"
    );
    assert!(
        fs::read_to_string(fx.snapshots().join("journal.log"))
            .unwrap()
            .contains(id.as_str())
    );

    // Özetler: değer yok, yol ~ ile.
    let detail = fx.app.show(&id).unwrap().unwrap();
    assert_eq!(detail.fragments.len(), 2);
    assert_eq!(detail.fragments[0].kind, "ini_key");
    assert_eq!(detail.fragments[1].before, "absent");
    assert_eq!(detail.conflicts, 0);
    assert!(detail.pinned);
    let latest = fx.app.latest(&sid("look.gtk")).unwrap().unwrap();
    assert_eq!(latest.id, id);
    assert_eq!(fx.app.original(&sid("look.gtk")).unwrap().unwrap().id, id);

    // Geri yükle: bayt bayt aynı, izin korunur.
    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    let RestoreOutcome::Restored { id: Some(rid) } = r else {
        panic!("{r:?}")
    };
    assert_eq!(fs::read_to_string(&f).unwrap(), GTK);
    assert_eq!(
        fs::metadata(&f).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let old = fx.app.show(&id).unwrap().unwrap();
    assert_eq!(old.state, "restored");
    assert_eq!(old.restored_by.as_ref(), Some(&rid));
    assert_eq!(
        fx.app.show(&rid).unwrap().unwrap().restores.as_ref(),
        Some(&id)
    );
    // Tekrar geri yükleme: zaten öncesinde.
    assert_eq!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: None }
    );
    // Geri yüklemenin geri alınması: uygulanmış hâle döner.
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&rid, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(fs::read_to_string(&f).unwrap(), now);
}

#[test]
fn created_file_is_removed_on_restore() {
    let fx = Fx::new();
    let f = fx.cfg("qt6ct/qt6ct.conf");
    let id = applied(fx.app.apply(change(
        "look.qt",
        vec![(ini(&f, "Appearance", "style"), text("Fusion"))],
    )));
    assert_eq!(
        fs::read_to_string(&f).unwrap(),
        "[Appearance]\nstyle=Fusion\n"
    );
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert!(!f.exists());
}

#[test]
fn three_way_conflicts() {
    let fx = Fx::new();
    let f = fx.cfg("settings.ini");
    fs::write(&f, GTK).unwrap();
    let id = applied(fx.app.apply(change(
        "look.gtk",
        vec![
            (ini(&f, "Settings", "gtk-theme-name"), text("Nuxut")),
            (ini(&f, "Settings", "gtk-font-name"), text("Inter 11")),
        ],
    )));
    // Kullanıcı sonradan bir anahtarı değiştirir.
    let edited = fs::read_to_string(&f)
        .unwrap()
        .replace("gtk-font-name=Inter 11", "gtk-font-name=Kendi 12");
    fs::write(&f, &edited).unwrap();
    assert_eq!(fx.app.list(&Filter::default()).unwrap()[0].conflicts, 1);

    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    let RestoreOutcome::Partial { conflicts, .. } = &r else {
        panic!("{r:?}")
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(
        conflicts[0].key.as_deref(),
        Some("[Settings] gtk-font-name")
    );
    let now = fs::read_to_string(&f).unwrap();
    assert!(
        now.contains("gtk-theme-name = Adwaita-dark") && now.contains("gtk-font-name=Kendi 12"),
        "{now}"
    );

    // Hepsi çakışırsa Conflict, hiçbir şey yazılmaz.
    let f2 = fx.cfg("b.ini");
    fs::write(&f2, "[S]\nk=1\n").unwrap();
    fx.advance(10_000);
    let id2 = applied(
        fx.app
            .apply(change("other", vec![(ini(&f2, "S", "k"), text("2"))])),
    );
    fs::write(&f2, "[S]\nk=3\n").unwrap();
    let r = fx.app.restore(&id2, RestoreMode::Safe);
    assert!(
        matches!(r, RestoreOutcome::Conflict { ref conflicts } if conflicts.len() == 1),
        "{r:?}"
    );
    assert_eq!(fs::read_to_string(&f2).unwrap(), "[S]\nk=3\n");
    // Force: üstüne yazar.
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id2, RestoreMode::Force),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(fs::read_to_string(&f2).unwrap(), "[S]\nk=1\n");
}

#[test]
fn coalescing_within_two_seconds() {
    let fx = Fx::new();
    let f = fx.cfg("settings.ini");
    fs::write(&f, GTK).unwrap();
    let k = ini(&f, "Settings", "gtk-theme-name");
    let id1 = applied(
        fx.app
            .apply(change("look.gtk", vec![(k.clone(), text("A"))])),
    );
    fx.advance(1500);
    let id2 = applied(
        fx.app
            .apply(change("look.gtk", vec![(k.clone(), text("B"))])),
    );
    assert_eq!(id1, id2);
    assert_eq!(fx.snap_count(), 1);
    // Birleşmiş işlemin geri yüklenmesi ilk before'a döner.
    // Pencere dışı: yeni snapshot.
    fx.advance(2500);
    let id3 = applied(
        fx.app
            .apply(change("look.gtk", vec![(k.clone(), text("C"))])),
    );
    assert_ne!(id3, id1);
    assert_eq!(fx.snap_count(), 2);
    // Aç-kapa (pencere içinde başa dönüş) iz bırakmaz.
    fx.advance(500);
    assert_eq!(
        fx.app
            .apply(change("look.gtk", vec![(k.clone(), text("B"))])),
        Outcome::Noop
    );
    assert_eq!(fx.snap_count(), 1);
    assert!(
        fs::read_to_string(&f)
            .unwrap()
            .contains("gtk-theme-name = B\n")
    );
    // Arada dış değişiklik varsa birleşmez.
    fx.advance(10_000);
    let id4 = applied(
        fx.app
            .apply(change("look.gtk", vec![(k.clone(), text("D"))])),
    );
    fs::write(&f, fs::read_to_string(&f).unwrap().replace("= D", "= Elle")).unwrap();
    fx.advance(500);
    let id5 = applied(
        fx.app
            .apply(change("look.gtk", vec![(k.clone(), text("E"))])),
    );
    assert_ne!(id4, id5);
    // İlk işlem geri yüklenince orijinal.
    fx.advance(10_000);
    let _ = fx.app.restore(&id5, RestoreMode::Safe);
    let _ = fx.app.restore(&id4, RestoreMode::Force);
    fx.advance(10_000);
    let r = fx.app.restore(&id1, RestoreMode::Force);
    assert!(matches!(r, RestoreOutcome::Restored { .. }), "{r:?}");
    assert_eq!(fs::read_to_string(&f).unwrap(), GTK);
}

#[test]
fn failed_write_rolls_back_earlier_files() {
    let fx = Fx::new();
    let a = fx.cfg("a.ini");
    let b = fx.cfg("b.ini");
    fs::write(&a, "[S]\nk=1\n").unwrap();
    fs::write(&b, "[S]\nk=1\n").unwrap();
    fs::set_permissions(&b, fs::Permissions::from_mode(0o444)).unwrap();
    let o = fx.app.apply(change(
        "multi",
        vec![
            (ini(&a, "S", "k"), text("2")),
            (ini(&b, "S", "k"), text("2")),
        ],
    ));
    assert!(
        matches!(o, Outcome::ExternallyManaged { ref target, reason: ExternalReason::ReadOnly } if target.ends_with("b.ini")),
        "{o:?}"
    );
    assert_eq!(fs::read_to_string(&a).unwrap(), "[S]\nk=1\n");
    assert_eq!(fs::read_to_string(&b).unwrap(), "[S]\nk=1\n");
    // Salt okunur hedef manifest yazılmadan elenir: iz yok.
    assert_eq!(fx.app.list(&Filter::default()).unwrap().len(), 0);
    assert_eq!(fx.app.latest(&sid("multi")).unwrap(), None);

    // Çözümleme hatası: hiçbir dosya yazılmaz.
    let t = fx.cfg("bad.toml");
    fs::write(&t, "x = [").unwrap();
    let o = fx.app.apply(change(
        "multi",
        vec![
            (ini(&a, "S", "k"), text("3")),
            (
                Fragment::TomlKey {
                    file: t.clone(),
                    key: KeyPath::dotted("a").unwrap(),
                },
                text("1"),
            ),
        ],
    ));
    assert_eq!(o, Outcome::Error { kind: "parse" });
    assert_eq!(fs::read_to_string(&a).unwrap(), "[S]\nk=1\n");
}

#[test]
fn busy_when_locked() {
    let fx = Fx::new();
    let f = fx.cfg("a.ini");
    fs::write(&f, "").unwrap();
    // Başka bir "süreç" kilidi tutuyor (ayrı açık dosya açıklaması).
    fs::create_dir_all(fx.root.join("state/nuxut")).unwrap();
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(fx.root.join("state/nuxut/apply.lock"))
        .unwrap();
    lock.lock().unwrap();
    assert_eq!(
        fx.app
            .apply(change("x", vec![(ini(&f, "S", "k"), text("1"))])),
        Outcome::Busy
    );
    assert_eq!(fs::read_to_string(&f).unwrap(), "");
    drop(lock);
    applied(
        fx.app
            .apply(change("x", vec![(ini(&f, "S", "k"), text("1"))])),
    );
}

#[test]
fn pending_becomes_interrupted_and_corrupt_is_skipped() {
    let fx = Fx::new();
    let f = fx.cfg("a.ini");
    let id = applied(
        fx.app
            .apply(change("x", vec![(ini(&f, "S", "k"), text("1"))])),
    );
    // Çökmeyi taklit et: manifest'i Pending'e çevir.
    let mp = fx.snapshots().join(id.as_str()).join("manifest.toml");
    let m = fs::read_to_string(&mp)
        .unwrap()
        .replace("state = \"applied\"", "state = \"pending\"");
    fs::write(&mp, m).unwrap();
    // Bozuk ve sahte girdiler.
    let bad = fx.snapshots().join("20261010T110000Z-dead");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("manifest.toml"), "gizli içerik = [").unwrap();
    fs::create_dir_all(fx.snapshots().join("../../../escape")).unwrap();
    fs::create_dir_all(fx.snapshots().join("not-an-id")).unwrap();
    fs::write(
        fx.snapshots().join("20261010T110000Z-beef"),
        "dosya, dizin değil",
    )
    .unwrap();

    let list = fx.app.list(&Filter::default()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].state, "interrupted");
    // Otomatik geri alınmadı.
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=1\n");
    // Yarım işlem geri alınabilir.
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert!(!f.exists());
    let unknown = SnapshotId::parse("20200101T000000Z-0000").unwrap();
    assert_eq!(
        fx.app.restore(&unknown, RestoreMode::Safe),
        RestoreOutcome::NotFound
    );
    assert_eq!(fx.app.show(&unknown).unwrap(), None);
}

#[test]
fn pruning_keeps_pinned_first_and_limits() {
    let mut fx = Fx::new();
    fx.app.set_limits(3, 6, TOTAL_BYTES_LIMIT);
    let f = fx.cfg("a.ini");
    fs::write(&f, "[S]\nk=orijinal\n").unwrap();
    let mut ids = Vec::new();
    for i in 0..8 {
        fx.advance(5_000);
        ids.push(applied(fx.app.apply(change(
            "one",
            vec![(ini(&f, "S", "k"), text(&format!("v{i}")))],
        ))));
    }
    let mine = fx
        .app
        .list(&Filter {
            setting: Some(sid("one")),
            limit: 200,
        })
        .unwrap();
    // 3 son + sabitlenmiş ilk.
    assert_eq!(mine.len(), 4, "{mine:?}");
    assert!(mine.iter().any(|s| s.id == ids[0] && s.pinned));
    assert_eq!(fx.app.original(&sid("one")).unwrap().unwrap().id, ids[0]);
    assert_eq!(fx.app.latest(&sid("one")).unwrap().unwrap().id, ids[7]);
    // "Nuxut öncesine dön" hâlâ çalışır.
    fx.advance(5_000);
    let r = fx.app.restore(&ids[0], RestoreMode::Force);
    assert!(matches!(r, RestoreOutcome::Restored { .. }), "{r:?}");
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=orijinal\n");

    // Toplam sınır: başka ayarlarla doldur.
    for i in 0..6 {
        fx.advance(5_000);
        let g = fx.cfg(&format!("o{i}.ini"));
        fs::write(&g, "[S]\nk=0\n").unwrap();
        applied(fx.app.apply(change(
            &format!("other{i}"),
            vec![(ini(&g, "S", "k"), text("1"))],
        )));
    }
    // Sabitlenmişler silinmez; toplam yine de sınıra yakın kalır.
    let all = fx
        .app
        .list(&Filter {
            setting: None,
            limit: 200,
        })
        .unwrap();
    let unpinned = all.iter().filter(|s| !s.pinned).count();
    assert!(all.len() <= 6 || unpinned == 0, "{all:?}");
    assert!(all.iter().any(|s| s.id == ids[0]));
}

#[test]
fn guard_and_store_failures() {
    // Sonda gerçek arka uç (`SystemBackend`, var olmayan program) alt süreç başlatmayı dener.
    let fx = Fx::exclusive();
    let f = fx.cfg("a.ini");
    // Test örneği kullanıcının state'iyle: hiçbir şey.
    let mut env = fx.app.env().clone();
    // Gerçek ev dizini içinde (var olmayan) bir state kökü: $HOME geçici olsa da reddedilir.
    let inside_home = crate::env::real_home()
        .expect("gerçek ev dizini")
        .join(".nuxut-apply-test-yok/state");
    env.state_home = Some(inside_home.clone());
    let app = Applier::open(env);
    assert_eq!(
        app.apply(change("x", vec![(ini(&f, "S", "k"), text("1"))])),
        Outcome::Skipped {
            reason: Skip::TestInstance
        }
    );
    assert!(!f.exists());
    assert!(!inside_home.exists());
    // Depo kurulamıyor: dış dosyaya yazılmaz.
    let mut env = fx.app.env().clone();
    let blocker = fx.root.join("blocker");
    fs::write(&blocker, "").unwrap();
    env.state_home = Some(blocker);
    let app = Applier::open(env);
    assert_eq!(
        app.apply(change("x", vec![(ini(&f, "S", "k"), text("1"))])),
        Outcome::SnapshotUnavailable
    );
    assert!(!f.exists());
    // dconf / systemd: test örneğinde atlanır.
    let d = change(
        "look.gtk4",
        vec![(
            Fragment::Dconf {
                key: "/org/gnome/desktop/interface/gtk-theme".into(),
            },
            text("'x'"),
        )],
    );
    assert_eq!(
        fx.app.apply(d.clone()),
        Outcome::Skipped {
            reason: Skip::TestInstance
        }
    );
    // Kullanıcı ortamında adaptör yoksa Unavailable.
    let mut user_env = fx.app.env().clone();
    user_env.test_instance = false;
    let app = Applier::open(user_env);
    assert_eq!(
        app.apply(d),
        Outcome::Unavailable {
            target: "dconf".into()
        }
    );
    // Geçersiz değişiklik.
    assert_eq!(
        fx.app.apply(change("x", vec![])),
        Outcome::Error { kind: "invalid" }
    );
}

#[test]
fn line_block_and_managed_file() {
    let fx = Fx::new();
    let hypr = fx.cfg("hypr/hyprland.conf");
    let line = Fragment::Line {
        file: hypr.clone(),
        marker: "source = ~/.config/hypr/nuxut.conf".into(),
        at: Position::End,
    };
    // Ana dosya yok: yaratılmaz.
    let o = fx.app.apply(change(
        "hypr.source",
        vec![(line.clone(), text("source = ~/.config/hypr/nuxut.conf"))],
    ));
    assert_eq!(o, Outcome::Error { kind: "not_found" });
    assert!(!hypr.exists());
    fs::create_dir_all(hypr.parent().unwrap()).unwrap();
    let original = "# kullanıcı\nmonitor=,preferred,auto,1\nbind = SUPER, Q, killactive";
    fs::write(&hypr, original).unwrap();
    let block = Fragment::Block {
        file: hypr.clone(),
        id: "env".into(),
    };
    let id = applied(fx.app.apply(change(
        "hypr.source",
        vec![
            (line.clone(), text("source = ~/.config/hypr/nuxut.conf")),
            (block, text("env = A,1")),
        ],
    )));
    let now = fs::read_to_string(&hypr).unwrap();
    assert!(now.starts_with(original), "{now}");
    assert!(
        now.contains(
            "\nsource = ~/.config/hypr/nuxut.conf\n# >>> nuxut:env\nenv = A,1\n# <<< nuxut:env"
        ),
        "{now}"
    );
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(fs::read_to_string(&hypr).unwrap(), original);

    // Yönetilen dosya: yabancı içerik snapshot'a girer, geri yüklemede geri gelir.
    let managed = fx.cfg("hypr/nuxut.conf");
    fs::write(&managed, "kullanıcının eski dosyası\n").unwrap();
    let body = format!(
        "{}a = 1\n",
        edit::managed::header(edit::managed::CommentStyle::Hash, "Hyprland")
    );
    let mf = Fragment::ManagedFile {
        file: managed.clone(),
    };
    let c = change(
        "hypr.generated",
        vec![(
            mf.clone(),
            FragValue::File {
                bytes: body.clone().into_bytes(),
                mode: 0,
            },
        )],
    );
    let plan = fx.app.plan(&c).unwrap();
    assert!(plan.entries[0].foreign && plan.entries[0].changes);
    let id = applied(fx.app.apply(c.clone()));
    assert_eq!(fs::read_to_string(&managed).unwrap(), body);
    let plan = fx.app.plan(&c).unwrap();
    assert!(!plan.entries[0].changes && !plan.entries[0].foreign);
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(
        fs::read_to_string(&managed).unwrap(),
        "kullanıcının eski dosyası\n"
    );
    // Yoktan yaratılan yönetilen dosya geri yüklemede silinir.
    let fresh = fx.cfg("environment.d/nuxut.conf");
    let id = applied(fx.app.apply(change(
        "look.qt_env",
        vec![(
            Fragment::ManagedFile {
                file: fresh.clone(),
            },
            FragValue::File {
                bytes: b"# Nuxut \xe2\x80\x94 x\nA=1\n".to_vec(),
                mode: 0o600,
            },
        )],
    )));
    assert_eq!(
        fs::metadata(&fresh).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert!(!fresh.exists());
}

#[test]
fn plan_detects_foreign_values() {
    let fx = Fx::new();
    let f = fx.cfg("settings.ini");
    fs::write(&f, GTK).unwrap();
    let k = ini(&f, "Settings", "gtk-theme-name");
    let c = change(
        "look.gtk",
        vec![
            (k.clone(), text("Nuxut")),
            (ini(&f, "Settings", "yeni"), text("1")),
        ],
    );
    let p = fx.app.plan(&c).unwrap();
    assert!(p.entries[0].foreign && p.entries[0].changes && p.entries[0].current == "set");
    assert!(!p.entries[1].foreign && p.entries[1].current == "absent");
    // Plan hiçbir şey yazmaz.
    assert_eq!(fs::read_to_string(&f).unwrap(), GTK);
    applied(fx.app.apply(c));
    // Nuxut'un yazdığı değerin üzerine yazmak yabancı değil.
    let p = fx
        .app
        .plan(&change("look.gtk", vec![(k, text("Başka"))]))
        .unwrap();
    assert!(!p.entries[0].foreign && p.entries[0].changes);
}

#[test]
fn toml_fragment_keeps_comments() {
    let fx = Fx::new();
    let f = fx.cfg("nuxut/shell.toml");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    let doc = "# kabuk\n[bar]\nheight = 32 # piksel\n";
    fs::write(&f, doc).unwrap();
    let key = Fragment::TomlKey {
        file: f.clone(),
        key: KeyPath::dotted("bar.height").unwrap(),
    };
    let id = applied(
        fx.app
            .apply(change("migration.shell", vec![(key.clone(), text(" 40 "))])),
    );
    assert_eq!(
        fs::read_to_string(&f).unwrap(),
        "# kabuk\n[bar]\nheight = 40 # piksel\n"
    );
    // Aynı değer farklı boşlukla: noop.
    fx.advance(10_000);
    assert_eq!(
        fx.app
            .apply(change("migration.shell", vec![(key.clone(), text("40"))])),
        Outcome::Noop
    );
    assert_eq!(
        fx.app
            .apply(change("migration.shell", vec![(key, text("not toml"))])),
        Outcome::Error { kind: "invalid" }
    );
    let _ = fx.app.restore(&id, RestoreMode::Safe);
    assert_eq!(fs::read_to_string(&f).unwrap(), doc);
}

#[test]
fn outcomes_serialize_without_values() {
    let id = SnapshotId::parse("20261010T120000Z-3fa1").unwrap();
    let s = toml::to_string(&Outcome::Applied { id: id.clone() }).unwrap();
    assert!(
        s.contains("result = \"applied\"") && s.contains("20261010T120000Z-3fa1"),
        "{s}"
    );
    let s = toml::to_string(&Outcome::Skipped {
        reason: Skip::TestInstance,
    })
    .unwrap();
    assert!(s.contains("reason = \"test_instance\""), "{s}");
    assert_eq!(
        toml::to_string(&Outcome::Noop).unwrap().trim(),
        "result = \"noop\""
    );
    let s = toml::to_string(&RestoreOutcome::Conflict { conflicts: vec![] }).unwrap();
    assert!(s.contains("result = \"conflict\""), "{s}");
    let o = Outcome::ExternallyManaged {
        target: "~/gizli/yol".into(),
        reason: ExternalReason::ReadOnly,
    };
    assert!(!format!("{o:?}").contains("gizli"));
    let fs = FragmentSummary {
        kind: "ini_key",
        target: "~/gizli".into(),
        key: None,
        before: "set",
        after: "absent",
    };
    assert!(!format!("{fs:?}").contains("gizli"));
}

#[test]
fn symlinked_target_and_two_paths_to_same_file() {
    let fx = Fx::new();
    let real = fx.cfg("dotfiles/settings.ini");
    fs::create_dir_all(real.parent().unwrap()).unwrap();
    fs::write(&real, GTK).unwrap();
    let link = fx.cfg("settings.ini");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    // Aynı dosyaya iki yoldan iki parça: tek okuma, tek yazım.
    let id = applied(fx.app.apply(change(
        "look.gtk",
        vec![
            (ini(&link, "Settings", "gtk-theme-name"), text("N")),
            (ini(&real, "Settings", "gtk-font-name"), text("F")),
        ],
    )));
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    let now = fs::read_to_string(&real).unwrap();
    assert!(now.contains("gtk-theme-name = N") && now.contains("gtk-font-name=F"));
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(fs::read_to_string(&real).unwrap(), GTK);
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn now_is_monotonic_enough() {
    // `generate` aynı saniyede farklı kimlik üretir.
    let t = SystemTime::now();
    let a = SnapshotId::generate(t);
    let b = SnapshotId::generate(t);
    let c = SnapshotId::generate(t);
    assert!(a != b || b != c);
}

// ---------------------------------------------------------------------------------------------
// Kilit yarışı, dış değişiklik, çökme kurtarma

#[test]
fn concurrent_appliers_serialize_without_lost_updates() {
    let fx = Fx::new();
    let f = fx.cfg("settings.ini");
    fs::write(&f, GTK).unwrap();
    let env = fx.app.env().clone();
    // Ayrı Applier'lar = ayrı açık dosya açıklamaları (ayrı süreçler gibi flock'ta yarışırlar).
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let env = env.clone();
            let f = f.clone();
            std::thread::spawn(move || {
                let app = Applier::open(env).with_backend(Box::new(crate::NoBackend));
                app.apply(change(
                    &format!("look.k{i}"),
                    vec![(ini(&f, "Settings", &format!("k{i}")), text(&i.to_string()))],
                ))
            })
        })
        .collect();
    for h in handles {
        let o = h.join().unwrap();
        assert!(matches!(o, Outcome::Applied { .. }), "{o:?}");
    }
    let now = fs::read_to_string(&f).unwrap();
    for i in 0..8 {
        assert!(now.contains(&format!("k{i}={i}\n")), "{now}");
    }
    assert!(now.starts_with(GTK));
    assert_eq!(fx.snap_count(), 8);
    // Hepsini geri al (herhangi bir sırada): dosya bayt bayt eski hâline döner.
    fx.advance(10_000);
    for s in fx.app.list(&Filter::default()).unwrap() {
        let r = fx.app.restore(&s.id, RestoreMode::Safe);
        assert!(
            matches!(r, RestoreOutcome::Restored { id: Some(_) }),
            "{r:?}"
        );
    }
    assert_eq!(fs::read_to_string(&f).unwrap(), GTK);
}

#[test]
fn external_edit_during_apply_rolls_back_everything() {
    let fx = Fx::new();
    let a = fx.cfg("a.ini");
    let b = fx.cfg("b.ini");
    fs::write(&a, "[S]\nk=1\n").unwrap();
    fs::write(&b, "[S]\nk=1\n").unwrap();
    // `a` yazılırken kullanıcı `b`'yi düzenler → `b`'nin ön koşulu tutmaz.
    let bb = b.clone();
    crate::write::BEFORE_RECHECK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || fs::write(&bb, "[S]\nk=elle\n").unwrap()));
    });
    let o = fx.app.apply(change(
        "multi",
        vec![
            (ini(&a, "S", "k"), text("2")),
            (ini(&b, "S", "k"), text("2")),
        ],
    ));
    assert_eq!(o, Outcome::Changed);
    assert_eq!(fs::read_to_string(&a).unwrap(), "[S]\nk=1\n");
    assert_eq!(fs::read_to_string(&b).unwrap(), "[S]\nk=elle\n");
    assert_eq!(fx.snap_count(), 0);
    // Geçici dosya kalmadı.
    let leftovers: Vec<_> = fs::read_dir(fx.cfg(""))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn crash_mid_coalesced_apply_is_interrupted_and_restorable() {
    let fx = Fx::new();
    let f = fx.cfg("settings.ini");
    fs::write(&f, GTK).unwrap();
    let k = ini(&f, "Settings", "gtk-theme-name");
    let id = applied(
        fx.app
            .apply(change("look.gtk", vec![(k.clone(), text("A"))])),
    );
    let after_a = fs::read_to_string(&f).unwrap();
    fx.advance(500);
    // İkinci (birleşecek) uygulama yeniden adlandırmadan hemen önce "çöker".
    crate::write::BEFORE_RECHECK.with(|h| {
        *h.borrow_mut() = Some(Box::new(|| panic!("çökme benzetimi")));
    });
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fx.app
            .apply(change("look.gtk", vec![(k.clone(), text("B"))]))
    }));
    assert!(r.is_err());
    // Dosya A'da kaldı; manifest Pending (önceki after = A, yeni after = B).
    assert_eq!(fs::read_to_string(&f).unwrap(), after_a);
    let list = fx.app.list(&Filter::default()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id);
    assert_eq!(list[0].state, "interrupted");
    // Önceki after da "bizim": çakışma yok; güvenli geri alma orijinale döndürür.
    assert_eq!(list[0].conflicts, 0);
    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    assert!(
        matches!(r, RestoreOutcome::Restored { id: Some(_) }),
        "{r:?}"
    );
    assert_eq!(fs::read_to_string(&f).unwrap(), GTK);
}

#[test]
fn orphan_snapshot_dir_is_cleaned_under_lock() {
    let fx = Fx::new();
    let f = fx.cfg("a.ini");
    applied(
        fx.app
            .apply(change("x", vec![(ini(&f, "S", "k"), text("1"))])),
    );
    // Blob yazılırken çökmüş işlem: manifest yok.
    let orphan = fx.snapshots().join("20261010T110000Z-0bad");
    fs::create_dir_all(&orphan).unwrap();
    fs::write(orphan.join("0.blob"), "eski içerik").unwrap();
    // Bozuk manifest'li dizin silinmez (yalnızca atlanır).
    let corrupt = fx.snapshots().join("20261010T110000Z-0bae");
    fs::create_dir_all(&corrupt).unwrap();
    fs::write(corrupt.join("manifest.toml"), "x = [").unwrap();
    assert_eq!(fx.snap_count(), 1);
    assert!(!orphan.exists());
    assert!(corrupt.exists());
}

// ---------------------------------------------------------------------------------------------
// Depodan okunan veri güvenilmez

fn load_manifest(fx: &Fx, id: &SnapshotId) -> crate::store::Manifest {
    let p = fx.snapshots().join(id.as_str()).join("manifest.toml");
    toml::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

fn save_manifest(fx: &Fx, m: &crate::store::Manifest) {
    let p = fx.snapshots().join(m.id.as_str()).join("manifest.toml");
    fs::write(p, toml::to_string(m).unwrap()).unwrap();
}

fn blob_of(s: &crate::store::Stored) -> u32 {
    match s {
        crate::store::Stored::Text { blob, .. } | crate::store::Stored::File { blob, .. } => *blob,
        crate::store::Stored::Absent => panic!("blob yok"),
    }
}

/// Bir ayar uygular; (kimlik, dosya) döner. Dosya `[S]\nk=1\n` → `k=2`.
fn one_applied(fx: &Fx) -> (SnapshotId, PathBuf) {
    let f = fx.cfg("a.ini");
    fs::write(&f, "[S]\nk=1\n").unwrap();
    let id = applied(
        fx.app
            .apply(change("x", vec![(ini(&f, "S", "k"), text("2"))])),
    );
    fx.advance(10_000);
    (id, f)
}

#[test]
fn tampered_manifest_path_outside_allowed_roots_is_refused() {
    let fx = Fx::new();
    let (id, _) = one_applied(&fx);
    let victim = fx.root.join("outside/victim.ini");
    fs::create_dir_all(victim.parent().unwrap()).unwrap();
    fs::write(&victim, "[S]\nk=2\n").unwrap();
    let mut m = load_manifest(&fx, &id);
    m.fragments[0].fragment = ini(&victim, "S", "k");
    save_manifest(&fx, &m);
    let r = fx.app.restore(&id, RestoreMode::Force);
    assert_eq!(
        r,
        RestoreOutcome::Skipped {
            reason: Skip::TestInstance
        }
    );
    assert_eq!(fs::read_to_string(&victim).unwrap(), "[S]\nk=2\n");
}

#[test]
fn tampered_manifest_shapes_are_rejected_as_corrupt() {
    let fx = Fx::new();
    let (id, f) = one_applied(&fx);
    let good = load_manifest(&fx, &id);
    let dotdot = PathBuf::from(format!("{}/../a.ini", fx.cfg("x").display()));
    let mut cases: Vec<crate::store::Manifest> = Vec::new();
    // `..` içeren yol.
    let mut m = good.clone();
    m.fragments[0].fragment = ini(&dotdot, "S", "k");
    cases.push(m);
    // Göreli yol.
    let mut m = good.clone();
    m.fragments[0].fragment = ini(Path::new("a.ini"), "S", "k");
    cases.push(m);
    // INI parçasına dosya değeri (setuid izinli).
    let mut m = good.clone();
    m.fragments[0].before = crate::store::Stored::File {
        blob: 0,
        mode: 0o4755,
        hash: "0".repeat(64),
    };
    cases.push(m);
    // Bozuk özet biçimi.
    let mut m = good.clone();
    m.fragments[0].after = crate::store::Stored::Text {
        blob: 1,
        hash: "../../x".into(),
    };
    cases.push(m);
    // Aşırı blob indeksi.
    let mut m = good.clone();
    m.fragments[0].after = crate::store::Stored::Text {
        blob: u32::MAX,
        hash: "a".repeat(64),
    };
    cases.push(m);
    // Kendini geri yükleyen kayıt.
    let mut m = good.clone();
    m.restores = Some(id.clone());
    cases.push(m);
    // Yinelenen parça.
    let mut m = good.clone();
    m.fragments.push(m.fragments[0].clone());
    cases.push(m);
    for (i, m) in cases.iter().enumerate() {
        save_manifest(&fx, m);
        assert_eq!(
            fx.app.restore(&id, RestoreMode::Force),
            RestoreOutcome::NotFound,
            "durum {i}"
        );
        assert_eq!(fx.app.show(&id).unwrap(), None, "durum {i}");
        assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=2\n");
    }
    // Başka kimliğin dizinine kopyalanmış manifest (kimlik uyuşmazlığı).
    save_manifest(&fx, &good);
    let other = SnapshotId::parse("20261010T110000Z-aaaa").unwrap();
    let od = fx.snapshots().join(other.as_str());
    fs::create_dir_all(&od).unwrap();
    fs::copy(
        fx.snapshots().join(id.as_str()).join("manifest.toml"),
        od.join("manifest.toml"),
    )
    .unwrap();
    assert_eq!(
        fx.app.restore(&other, RestoreMode::Force),
        RestoreOutcome::NotFound
    );
    // Sağlam hâli hâlâ çalışır.
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=1\n");
}

#[test]
fn tampered_blobs_are_refused() {
    let fx = Fx::new();
    let (id, f) = one_applied(&fx);
    let m = load_manifest(&fx, &id);
    let dir = fx.snapshots().join(id.as_str());
    let before_blob = dir.join(format!("{}.blob", blob_of(&m.fragments[0].before)));
    let original = fs::read(&before_blob).unwrap();

    // 1) İçerik değişti, özet tutmuyor.
    fs::write(&before_blob, "9").unwrap();
    assert_eq!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Error {
            kind: "snapshot_unavailable"
        }
    );
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=2\n");

    // 2) Özeti de yeniden hesaplanmış satır sonu enjeksiyonu: değer kuralları reddeder.
    let evil = "1\n[Autostart]\nexec=kotu";
    fs::write(&before_blob, evil).unwrap();
    let mut m2 = m.clone();
    if let crate::store::Stored::Text { hash, .. } = &mut m2.fragments[0].before {
        *hash = blake3::hash(evil.as_bytes()).to_hex().to_string();
    }
    save_manifest(&fx, &m2);
    assert_eq!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Error {
            kind: "snapshot_unavailable"
        }
    );
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=2\n");

    // 3) Blob, depo dışındaki bir dosyaya symlink: izlenmez.
    save_manifest(&fx, &m);
    let secret = fx.root.join("secret");
    fs::write(&secret, &original).unwrap();
    fs::remove_file(&before_blob).unwrap();
    std::os::unix::fs::symlink(&secret, &before_blob).unwrap();
    assert_eq!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Error {
            kind: "snapshot_unavailable"
        }
    );
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=2\n");

    // Onarılınca çalışır.
    fs::remove_file(&before_blob).unwrap();
    fs::write(&before_blob, &original).unwrap();
    fs::set_permissions(&before_blob, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
}

#[test]
fn hostile_store_entries_do_not_hang_or_escape() {
    let fx = Fx::new();
    let (id, _) = one_applied(&fx);
    // Manifest bir FIFO: açılış takılmamalı (O_NONBLOCK + düz dosya denetimi).
    let fifo_id = SnapshotId::parse("20261010T100000Z-f1f0").unwrap();
    let fd = fx.snapshots().join(fifo_id.as_str());
    fs::create_dir_all(&fd).unwrap();
    // Alt süreç (`mkfifo`) değil, doğrudan çağrı: başka testlerin kilit fd'si çocuğa sızmaz.
    rustix::fs::mknodat(
        rustix::fs::CWD,
        fd.join("manifest.toml"),
        rustix::fs::FileType::Fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
        0,
    )
    .unwrap();
    // Depo dışına işaret eden snapshot dizini (symlink): listelenmez, kimlikle de açılmaz.
    let elsewhere = fx.root.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::copy(
        fx.snapshots().join(id.as_str()).join("manifest.toml"),
        elsewhere.join("manifest.toml"),
    )
    .unwrap();
    let link_id = SnapshotId::parse("20261010T100000Z-1111").unwrap();
    std::os::unix::fs::symlink(&elsewhere, fx.snapshots().join(link_id.as_str())).unwrap();
    // Aşırı büyük manifest.
    let big_id = SnapshotId::parse("20261010T100000Z-b16b").unwrap();
    let bd = fx.snapshots().join(big_id.as_str());
    fs::create_dir_all(&bd).unwrap();
    fs::write(
        bd.join("manifest.toml"),
        vec![b'#'; crate::store::MAX_MANIFEST_BYTES as usize + 1],
    )
    .unwrap();
    // Manifest bir symlink.
    let sl_id = SnapshotId::parse("20261010T100000Z-5115").unwrap();
    let sd = fx.snapshots().join(sl_id.as_str());
    fs::create_dir_all(&sd).unwrap();
    std::os::unix::fs::symlink(
        fx.snapshots().join(id.as_str()).join("manifest.toml"),
        sd.join("manifest.toml"),
    )
    .unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    let env = fx.app.env().clone();
    std::thread::spawn(move || {
        let app = Applier::open(env).with_backend(Box::new(crate::NoBackend));
        let n = app.list(&Filter::default()).map(|l| l.len());
        let _ = tx.send(n);
    });
    let n = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("list takıldı");
    assert_eq!(n.unwrap(), 1);
    for bad in [&fifo_id, &link_id, &big_id, &sl_id] {
        assert_eq!(fx.app.show(bad).unwrap(), None, "{bad}");
        assert_eq!(
            fx.app.restore(bad, RestoreMode::Force),
            RestoreOutcome::NotFound
        );
    }
    // Dışarıdaki kopyaya dokunulmadı; budama symlink'i izleyip dışarıyı silmez.
    assert!(elsewhere.join("manifest.toml").exists());
}

#[test]
fn journal_and_lock_are_private_and_not_followed() {
    let fx = Fx::new();
    // Depo önceden gevşek izinle var: sıkılaştırılır.
    fs::create_dir_all(fx.snapshots()).unwrap();
    fs::set_permissions(fx.snapshots(), fs::Permissions::from_mode(0o755)).unwrap();
    // journal.log kullanıcının başka bir dosyasına symlink: eklenmez.
    let target = fx.root.join("victim.log");
    fs::write(&target, "dokunma\n").unwrap();
    std::os::unix::fs::symlink(&target, fx.snapshots().join("journal.log")).unwrap();
    let f = fx.cfg("a.ini");
    applied(
        fx.app
            .apply(change("x", vec![(ini(&f, "S", "k"), text("1"))])),
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "dokunma\n");
    assert_eq!(
        fs::metadata(fx.snapshots()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let lock = fx.root.join("state/nuxut/apply.lock");
    assert_eq!(
        fs::metadata(&lock).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // Kilit dosyası symlink ise depo kullanılamaz (izlenmez).
    fs::remove_file(&lock).unwrap();
    std::os::unix::fs::symlink(&target, &lock).unwrap();
    fx.advance(10_000);
    assert_eq!(
        fx.app
            .apply(change("x", vec![(ini(&f, "S", "k"), text("2"))])),
        Outcome::SnapshotUnavailable
    );
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=1\n");
}

#[test]
fn pruning_by_total_bytes_drops_oldest_unpinned() {
    let mut fx = Fx::new();
    let f = fx.cfg("big.conf");
    let body = |c: char| {
        let mut s = edit::managed::header(edit::managed::CommentStyle::Hash, "test");
        s.push_str(&c.to_string().repeat(100 * 1024));
        s.into_bytes()
    };
    let mf = Fragment::ManagedFile { file: f.clone() };
    let mut ids = Vec::new();
    for c in ['a', 'b', 'c', 'd', 'e', 'f'] {
        fx.advance(5_000);
        ids.push(applied(fx.app.apply(change(
            "gen",
            vec![(
                mf.clone(),
                FragValue::File {
                    bytes: body(c),
                    mode: 0,
                },
            )],
        ))));
    }
    assert_eq!(fx.snap_count(), 6);
    // Her işlem ~200 KiB (önce + sonra). 500 KiB sınırı: sabitlenmiş ilk + en yeniler kalır.
    fx.app.set_limits(20, 500, 500 * 1024);
    fx.advance(5_000);
    let last = applied(fx.app.apply(change(
        "gen",
        vec![(
            mf.clone(),
            FragValue::File {
                bytes: body('g'),
                mode: 0,
            },
        )],
    )));
    let left: Vec<SnapshotId> = fx
        .app
        .list(&Filter {
            setting: None,
            limit: 200,
        })
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert!(
        left.contains(&ids[0]),
        "sabitlenmiş ilk silinmemeli: {left:?}"
    );
    assert!(left.contains(&last));
    assert!(
        !left.contains(&ids[1]) && !left.contains(&ids[2]),
        "{left:?}"
    );
    let total: u64 = left
        .iter()
        .map(|id| {
            fs::read_dir(fx.snapshots().join(id.as_str()))
                .unwrap()
                .map(|e| e.unwrap().metadata().unwrap().len())
                .sum::<u64>()
        })
        .sum();
    assert!(total <= 500 * 1024, "{total}");
}

#[test]
fn symlink_into_read_only_store_is_externally_managed() {
    let fx = Fx::new();
    let store = fx.cfg("store/settings.ini");
    fs::create_dir_all(store.parent().unwrap()).unwrap();
    fs::write(&store, GTK).unwrap();
    fs::set_permissions(&store, fs::Permissions::from_mode(0o444)).unwrap();
    let link = fx.cfg("gtk-3.0/settings.ini");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&store, &link).unwrap();
    let o = fx.app.apply(change(
        "look.gtk",
        vec![(ini(&link, "Settings", "gtk-theme-name"), text("N"))],
    ));
    assert!(matches!(o, Outcome::ExternallyManaged { .. }), "{o:?}");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&store).unwrap(), GTK);
    // Kırık symlink: yerine dosya konmaz.
    let broken = fx.cfg("gtk-4.0/settings.ini");
    fs::create_dir_all(broken.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(fx.root.join("yok"), &broken).unwrap();
    let o = fx.app.apply(change(
        "look.gtk4",
        vec![(ini(&broken, "Settings", "gtk-theme-name"), text("N"))],
    ));
    assert!(matches!(o, Outcome::ExternallyManaged { .. }), "{o:?}");
    assert!(
        fs::symlink_metadata(&broken)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

// ---------------------------------------------------------------------------------------------
// Devralma (Change::take_over; shell docs/design/look-external.md §4)

fn reason_of(o: &Outcome) -> Option<ExternalReason> {
    match o {
        Outcome::ExternallyManaged { reason, .. } => Some(*reason),
        _ => None,
    }
}

fn is_symlink(p: &Path) -> bool {
    fs::symlink_metadata(p).unwrap().file_type().is_symlink()
}

fn ino_mode_nlink(p: &Path) -> (u64, u32, u64) {
    use std::os::unix::fs::MetadataExt;
    let m = fs::symlink_metadata(p).unwrap();
    (m.ino(), m.mode() & 0o777, m.nlink())
}

#[test]
fn external_reasons_are_reported() {
    let fx = Fx::new();
    let ro = fx.cfg("ro.ini");
    fs::write(&ro, GTK).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o444)).unwrap();
    let a = fx.cfg("a.ini");
    let b = fx.cfg("b.ini");
    fs::write(&a, GTK).unwrap();
    fs::hard_link(&a, &b).unwrap();
    let broken = fx.cfg("broken.ini");
    std::os::unix::fs::symlink(fx.cfg("yok/yok.ini"), &broken).unwrap();
    let ok = fx.cfg("ok.ini");
    fs::write(&ok, GTK).unwrap();
    let link_ok = fx.cfg("link-ok.ini");
    std::os::unix::fs::symlink(&ok, &link_ok).unwrap();
    let locked = fx.cfg("locked");
    fs::create_dir_all(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();

    assert_eq!(external_reason(&ro), Some(ExternalReason::ReadOnly));
    assert_eq!(external_reason(&a), Some(ExternalReason::HardLink));
    assert_eq!(external_reason(&broken), Some(ExternalReason::SymlinkBroken));
    assert_eq!(external_reason(&ok), None);
    assert_eq!(external_reason(&link_ok), None);
    assert_eq!(external_reason(&fx.cfg("yeni/yeni.ini")), None);
    assert_eq!(
        external_reason(&locked.join("x.ini")),
        Some(ExternalReason::DirNotWritable)
    );
    for (p, r) in [
        (&ro, ExternalReason::ReadOnly),
        (&a, ExternalReason::HardLink),
        (&broken, ExternalReason::SymlinkBroken),
    ] {
        let o = fx
            .app
            .apply(change("look.gtk", vec![(ini(p, "Settings", "gtk-theme-name"), text("N"))]));
        assert_eq!(reason_of(&o), Some(r), "{o:?}");
        assert!(r.can_take_over(p));
    }
    let o = fx.app.apply(change(
        "look.gtk",
        vec![(
                Fragment::ManagedFile { file: locked.join("x.css") },
                FragValue::File { bytes: b"x".to_vec(), mode: 0 },
            )],
    ));
    assert_eq!(reason_of(&o), Some(ExternalReason::DirNotWritable), "{o:?}");
    // Yazılamayan dizin devralınamaz.
    let o = fx.app.apply(
        change(
            "look.gtk",
            vec![(
                Fragment::ManagedFile { file: locked.join("x.css") },
                FragValue::File { bytes: b"x".to_vec(), mode: 0 },
            )],
        )
        .take_over(locked.join("x.css")),
    );
    assert_eq!(reason_of(&o), Some(ExternalReason::DirNotWritable), "{o:?}");
    let json = toml::to_string(&o).unwrap();
    assert!(json.contains("reason = \"dir_not_writable\""), "{json}");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn take_over_symlink_and_restore_relinks() {
    let fx = Fx::new();
    let store = fx.cfg("dotfiles/settings.ini");
    fs::create_dir_all(store.parent().unwrap()).unwrap();
    fs::write(&store, GTK).unwrap();
    fs::set_permissions(&store, fs::Permissions::from_mode(0o444)).unwrap();
    let link = fx.cfg("gtk-3.0/settings.ini");
    fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("../dotfiles/settings.ini", &link).unwrap();
    let key = ini(&link, "Settings", "gtk-theme-name");

    let plan = fx
        .app
        .plan(&change("look.gtk", vec![(key.clone(), text("N"))]).take_over(&link))
        .unwrap();
    assert!(plan.entries.iter().any(|e| e.fragment.kind == "file_form" && e.changes));
    let id = applied(
        fx.app
            .apply(change("look.gtk", vec![(key.clone(), text("N"))]).take_over(&link)),
    );
    // Yol artık Nuxut'un düz dosyası; hedef (dotfile) dokunulmamış.
    assert!(!is_symlink(&link));
    assert_eq!(ino_mode_nlink(&link).1, 0o644);
    let got = fs::read_to_string(&link).unwrap();
    assert!(got.contains("gtk-theme-name = N") || got.contains("gtk-theme-name=N"), "{got}");
    assert!(got.starts_with("# kullanıcının yorumu\n"), "{got}");
    assert_eq!(fs::read_to_string(&store).unwrap(), GTK);
    let d = fx.app.show(&id).unwrap().unwrap();
    assert!(d.fragments.iter().any(|f| f.kind == "file_form"));
    // Sonraki uygulama devralmasız da yazabilir (artık dışarıdan yönetilmiyor).
    fx.advance(10_000);
    applied(fx.app.apply(change("look.gtk", vec![(key.clone(), text("M"))])));

    // Önceki hale dön (devralmanın işlemi): symlink aynı göreli hedef metniyle geri gelir.
    fx.advance(10_000);
    let latest = fx.app.latest(&sid("look.gtk")).unwrap().unwrap();
    let r = fx.app.restore(&latest.id, RestoreMode::Safe);
    assert!(matches!(r, RestoreOutcome::Restored { id: Some(_) }), "{r:?}");
    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    assert!(matches!(r, RestoreOutcome::Restored { id: Some(_) }), "{r:?}");
    assert!(is_symlink(&link));
    assert_eq!(
        fs::read_link(&link).unwrap(),
        PathBuf::from("../dotfiles/settings.ini")
    );
    assert_eq!(fs::read_to_string(&link).unwrap(), GTK);
    assert_eq!(ino_mode_nlink(&store).1, 0o444);
    // Geri yüklemenin geri yüklenmesi yeniden devralır.
    fx.advance(10_000);
    let back = fx.app.latest(&sid("look.gtk")).unwrap().unwrap();
    let r = fx.app.restore(&back.id, RestoreMode::Safe);
    assert!(matches!(r, RestoreOutcome::Restored { id: Some(_) }), "{r:?}");
    assert!(!is_symlink(&link));
    assert!(fs::read_to_string(&link).unwrap().contains('N'));
}

/// Gerçek senaryo (2026-10-16): `~/.config/gtk-4.0/gtk.css` →
/// `/usr/share/themes/Dracula/gtk-4.0/gtk.css` (başka sahip). Bağın kendisi devralınır, hedefe
/// dokunulmaz; geri yükleme bağı aynı metinle geri koyar.
#[test]
fn take_over_symlink_to_foreign_file_and_restore_relinks() {
    use std::os::unix::fs::MetadataExt;
    let Some(target) = ["/usr/lib/os-release", "/etc/os-release", "/etc/passwd"]
        .into_iter()
        .map(Path::new)
        .find(|p| {
            fs::metadata(p).is_ok_and(|m| m.is_file() && m.uid() != rustix::process::geteuid().as_raw())
        })
    else {
        eprintln!("başka kullanıcıya ait okunur dosya yok; atlandı");
        return;
    };
    let original = fs::read(target).unwrap();
    let fx = Fx::new();
    let css = fx.cfg("gtk-4.0/gtk.css");
    fs::create_dir_all(css.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(target, &css).unwrap();
    assert_eq!(external_reason(&css), Some(ExternalReason::NotOwned));
    assert!(ExternalReason::NotOwned.can_take_over(&css));
    let line = Fragment::Line {
        file: css.clone(),
        marker: "@import 'nuxut.css';".into(),
        at: Position::Start,
    };
    let req = || change("look.css", vec![(line.clone(), text("@import 'nuxut.css';"))]);
    // Devralmasız: bağ izlenir, hedefe yazılmaz (test örneğinde kök dışı → atlanır).
    let o = fx.app.apply(req());
    assert!(!matches!(o, Outcome::Applied { .. }), "{o:?}");
    assert!(is_symlink(&css));

    let id = applied(fx.app.apply(req().take_over(&css)));
    assert!(!is_symlink(&css));
    let got = fs::read(&css).unwrap();
    assert!(got.starts_with(b"@import 'nuxut.css';\n"));
    assert_eq!(&got[b"@import 'nuxut.css';\n".len()..], &original[..]);
    assert_eq!(ino_mode_nlink(&css).1 & 0o200, 0o200);
    assert_eq!(fs::read(target).unwrap(), original);
    // Artık Nuxut'un dosyası: yeniden uygulama no-op.
    fx.advance(10_000);
    assert!(matches!(fx.app.apply(req().take_over(&css)), Outcome::Noop), "tekrar");

    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    assert!(matches!(r, RestoreOutcome::Restored { id: Some(_) }), "{r:?}");
    assert!(is_symlink(&css));
    assert_eq!(fs::read_link(&css).unwrap(), target);
    assert_eq!(fs::read(target).unwrap(), original);

    // Başka kullanıcıya ait düz dosya (bağ değil) devralınamaz.
    let _ = (|| {
        let copy = fx.cfg("foreign.css");
        fs::hard_link(target, &copy).ok()?; // protected_hardlinks çoğunlukla engeller
        let line = Fragment::Line {
            file: copy.clone(),
            marker: "@import 'nuxut.css';".into(),
            at: Position::Start,
        };
        let o = fx.app.apply(
            change("look.css", vec![(line, text("@import 'nuxut.css';"))]).take_over(&copy),
        );
        assert_eq!(reason_of(&o), Some(ExternalReason::NotOwned), "{o:?}");
        Some(())
    })();
}

/// Gerçek vaka (2026-10-16): `gtk-4.0/{gtk.css,gtk-dark.css,assets}` bir temanın dosyalarına
/// sembolik bağ. İki css bağı tek işlemde devralınır (bağlar snapshot'ta), `assets` dizin bağına
/// dokunulmaz; geri yükleme iki bağı aynı göreli metinle geri koyar, tema dosyaları değişmez.
#[test]
fn take_over_theme_symlinks_css_pair_and_restore() {
    let fx = Fx::new();
    let theme = fx.cfg("themes/Dracula/gtk-4.0");
    fs::create_dir_all(theme.join("assets")).unwrap();
    let light = "window { color: #f8f8f2; }\n";
    let dark = "window { color: #282a36; }\n";
    fs::write(theme.join("gtk.css"), light).unwrap();
    fs::write(theme.join("gtk-dark.css"), dark).unwrap();
    fs::write(theme.join("assets/check.png"), b"png").unwrap();
    for f in ["gtk.css", "gtk-dark.css"] {
        fs::set_permissions(theme.join(f), fs::Permissions::from_mode(0o444)).unwrap();
    }
    let dir = fx.cfg("gtk-4.0");
    fs::create_dir_all(&dir).unwrap();
    let rel = |n: &str| format!("../themes/Dracula/gtk-4.0/{n}");
    for n in ["gtk.css", "gtk-dark.css", "assets"] {
        std::os::unix::fs::symlink(rel(n), dir.join(n)).unwrap();
    }
    let import = "@import url(\"nuxut.css\");";
    let line = |n: &str| Fragment::Line {
        file: dir.join(n),
        marker: import.into(),
        at: Position::End,
    };
    let req = change(
        "look.gtk4",
        vec![(line("gtk.css"), text(import)), (line("gtk-dark.css"), text(import))],
    )
    .take_over(dir.join("gtk.css"))
    .take_over(dir.join("gtk-dark.css"));
    let id = applied(fx.app.apply(req));

    for (n, orig) in [("gtk.css", light), ("gtk-dark.css", dark)] {
        let p = dir.join(n);
        assert!(!is_symlink(&p), "{n}");
        assert_eq!(fs::read_to_string(&p).unwrap(), format!("{orig}{import}\n"), "{n}");
        assert_eq!(fs::read_to_string(theme.join(n)).unwrap(), orig, "{n}");
    }
    assert!(is_symlink(&dir.join("assets")));
    let d = fx.app.show(&id).unwrap().unwrap();
    assert_eq!(d.fragments.iter().filter(|f| f.kind == "file_form").count(), 2);

    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    assert!(matches!(r, RestoreOutcome::Restored { id: Some(_) }), "{r:?}");
    for (n, orig) in [("gtk.css", light), ("gtk-dark.css", dark), ("assets", "")] {
        let p = dir.join(n);
        assert!(is_symlink(&p), "{n}");
        assert_eq!(fs::read_link(&p).unwrap(), PathBuf::from(rel(n)), "{n}");
        if !orig.is_empty() {
            assert_eq!(fs::read_to_string(&p).unwrap(), orig, "{n}");
            assert_eq!(ino_mode_nlink(&theme.join(n)).1, 0o444, "{n}");
        }
    }
    assert_eq!(fs::read(theme.join("assets/check.png")).unwrap(), b"png");
}

#[test]
fn take_over_broken_symlink_and_read_only_file() {
    let fx = Fx::new();
    let broken = fx.cfg("gtk-4.0/gtk.css");
    fs::create_dir_all(broken.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("/hm/yok/gtk.css", &broken).unwrap();
    let line = Fragment::Line {
        file: broken.clone(),
        marker: "@import 'nuxut.css';".into(),
        at: Position::Start,
    };
    let id = applied(fx.app.apply(
        change("look.css", vec![(line, text("@import 'nuxut.css';"))]).take_over(&broken),
    ));
    assert!(!is_symlink(&broken));
    assert_eq!(fs::read_to_string(&broken).unwrap(), "@import 'nuxut.css';\n");
    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    assert!(matches!(r, RestoreOutcome::Restored { id: Some(_) }), "{r:?}");
    assert_eq!(fs::read_link(&broken).unwrap(), PathBuf::from("/hm/yok/gtk.css"));

    // Salt okunur: u+w ile yazılır, geri yüklemede içerik ve izin geri gelir.
    let ro = fx.cfg("qt5ct/qt5ct.conf");
    fs::create_dir_all(ro.parent().unwrap()).unwrap();
    fs::write(&ro, GTK).unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o440)).unwrap();
    let key = ini(&ro, "Settings", "gtk-theme-name");
    let id = applied(
        fx.app
            .apply(change("look.qt", vec![(key, text("N"))]).take_over(&ro)),
    );
    assert_eq!(ino_mode_nlink(&ro).1, 0o640);
    assert!(fs::read_to_string(&ro).unwrap().contains('N'));
    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    assert!(matches!(r, RestoreOutcome::Restored { id: Some(_) }), "{r:?}");
    assert_eq!(fs::read_to_string(&ro).unwrap(), GTK);
    assert_eq!(ino_mode_nlink(&ro).1, 0o440);
}

#[test]
fn take_over_hard_link_and_restore_relinks() {
    let fx = Fx::new();
    let ours = fx.cfg("kdeglobals");
    let theirs = fx.cfg("other-tool/kdeglobals");
    fs::create_dir_all(theirs.parent().unwrap()).unwrap();
    fs::write(&theirs, GTK).unwrap();
    fs::hard_link(&theirs, &ours).unwrap();
    let (ino, _, _) = ino_mode_nlink(&theirs);
    let key = ini(&ours, "Settings", "gtk-theme-name");
    let id = applied(
        fx.app
            .apply(change("look.kde", vec![(key, text("N"))]).take_over(&ours)),
    );
    // Bağ koptu: öbür yol eski içerikte; inode depoda tutuluyor (öbür yol + depo bağı).
    assert_ne!(ino_mode_nlink(&ours).0, ino);
    assert_eq!(ino_mode_nlink(&ours).2, 1);
    assert_eq!(fs::read_to_string(&theirs).unwrap(), GTK);
    assert_eq!(ino_mode_nlink(&theirs).2, 2);
    assert!(fs::read_to_string(&ours).unwrap().contains('N'));
    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    assert!(matches!(r, RestoreOutcome::Restored { id: Some(_) }), "{r:?}");
    // Sabit bağ yeniden kuruldu: aynı inode.
    assert_eq!(ino_mode_nlink(&ours).0, ino);
    assert_eq!(fs::read_to_string(&ours).unwrap(), GTK);
}

#[test]
fn take_over_rules() {
    let fx = Fx::new();
    let ok = fx.cfg("ok.ini");
    fs::write(&ok, GTK).unwrap();
    let key = ini(&ok, "Settings", "gtk-theme-name");
    // Dışarıdan yönetilmeyen dosyada etkisiz: biçim kaydı yok.
    let id = applied(
        fx.app
            .apply(change("look.gtk", vec![(key.clone(), text("N"))]).take_over(&ok)),
    );
    let d = fx.app.show(&id).unwrap().unwrap();
    assert!(d.fragments.iter().all(|f| f.kind != "file_form"));
    // İstekte olmayan dosya devralınamaz; biçim parçası istekte verilemez.
    let o = fx.app.apply(
        change("look.gtk", vec![(key.clone(), text("M"))]).take_over(fx.cfg("baska.ini")),
    );
    assert!(matches!(o, Outcome::Error { kind: "invalid" }), "{o:?}");
    let o = fx.app.apply(change(
        "look.gtk",
        vec![(Fragment::FileForm { file: ok.clone() }, text("regular"))],
    ));
    assert!(matches!(o, Outcome::Error { kind: "invalid" }), "{o:?}");
    // Biçim değerleri.
    for (t, ok) in [
        ("regular", true),
        ("hardlink", true),
        ("mode:444", true),
        ("mode:644", false),
        ("mode:4444", false),
        ("symlink:../a", true),
        ("symlink:", false),
        ("symlink:a\nb", false),
        ("x", false),
    ] {
        assert_eq!(FileForm::parse(t).is_some(), ok, "{t}");
    }
    assert!(!format!("{:?}", FileForm::Symlink("/gizli".into())).contains("gizli"));
}

// ---------------------------------------------------------------------------------------------
// dconf / systemd: sahte arka uçla (gerçek oturuma asla dokunulmaz)

#[derive(Default)]
struct Fake {
    values: std::sync::Mutex<HashMap<String, FragValue>>,
    fail_writes: std::sync::atomic::AtomicBool,
    on_write: std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl crate::Backend for Arc<Fake> {
    fn read(&self, f: &Fragment) -> Result<FragValue, ApplyError> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .get(&format!("{f:?}"))
            .cloned()
            .unwrap_or(FragValue::Absent))
    }
    fn write(&self, f: &Fragment, v: &FragValue) -> Result<(), ApplyError> {
        if let Some(hook) = self.on_write.lock().unwrap().take() {
            hook();
        }
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(ApplyError::Io(std::io::ErrorKind::Other));
        }
        let mut m = self.values.lock().unwrap();
        if v.is_absent() {
            m.remove(&format!("{f:?}"));
        } else {
            m.insert(format!("{f:?}"), v.clone());
        }
        Ok(())
    }
}

/// Kullanıcı ortamı gibi (test örneği değil) ama tüm kökler geçici; arka uç sahte.
fn user_fx() -> (Fx, Arc<Fake>) {
    let mut fx = Fx::new();
    let mut env = fx.app.env().clone();
    env.test_instance = false;
    let fake = Arc::new(Fake::default());
    let mut app = Applier::open(env).with_backend(Box::new(fake.clone()));
    let c = fx.clock.clone();
    app.set_clock(Box::new(move || {
        UNIX_EPOCH + Duration::from_millis(c.load(Ordering::SeqCst))
    }));
    app.set_lock_wait(Duration::from_millis(100));
    fx.app = app;
    (fx, fake)
}

fn dconf(key: &str) -> Fragment {
    Fragment::Dconf { key: key.into() }
}

/// `dconf read` GLib biçiminde döndürür (`1.00` → `1.0`): aynı değer yeniden yazılmaz
/// (shell look-external.md §1, saniyede bir yeniden yazım döngüsü).
#[test]
fn dconf_values_compare_as_gvariant() {
    let (fx, fake) = user_fx();
    let scale = dconf("/org/gnome/desktop/interface/text-scaling-factor");
    let theme = dconf("/org/gnome/desktop/interface/gtk-theme");
    fake.values.lock().unwrap().insert(format!("{scale:?}"), text("1.0"));
    fake.values.lock().unwrap().insert(format!("{theme:?}"), text("'Adwaita'"));
    let c = change(
        "look.fonts",
        vec![(scale.clone(), text("1.00")), (theme.clone(), text("\"Adwaita\""))],
    );
    let plan = fx.app.plan(&c).unwrap();
    assert!(plan.entries.iter().all(|e| !e.changes), "{plan:?}");
    assert_eq!(fx.app.apply(c), Outcome::Noop);
    assert_eq!(fake.read(&scale).unwrap(), text("1.0"));
    // Gerçek değişiklik yine yazılır.
    let c = change("look.fonts", vec![(scale.clone(), text("1.10"))]);
    applied(fx.app.apply(c));
    assert_eq!(fake.read(&scale).unwrap(), text("1.10"));
}

#[test]
fn dconf_and_unit_apply_restore_and_conflict() {
    let (fx, fake) = user_fx();
    let theme = dconf("/org/gnome/desktop/interface/gtk-theme");
    let scheme = dconf("/org/gnome/desktop/interface/color-scheme");
    let unit = Fragment::UnitState {
        unit: "nuxut-shell.service".into(),
    };
    fake.values
        .lock()
        .unwrap()
        .insert(format!("{theme:?}"), text("'Adwaita'"));
    let id = applied(fx.app.apply(change(
        "look.gtk4",
        vec![
            (theme.clone(), text("'Nuxut'")),
            (scheme.clone(), text("'prefer-dark'")),
            (unit.clone(), text("enabled")),
        ],
    )));
    assert_eq!(fake.read(&theme).unwrap(), text("'Nuxut'"));
    assert_eq!(fake.read(&unit).unwrap(), text("enabled"));
    let d = fx.app.show(&id).unwrap().unwrap();
    assert_eq!(
        d.fragments[0].target,
        "/org/gnome/desktop/interface/gtk-theme"
    );
    assert_eq!(d.fragments[1].before, "absent");

    // Kullanıcı color-scheme'i sonradan değiştirir → çakışma, diğerleri geri gelir.
    fake.values
        .lock()
        .unwrap()
        .insert(format!("{scheme:?}"), text("'default'"));
    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    assert!(
        matches!(&r, RestoreOutcome::Partial { conflicts, .. } if conflicts.len() == 1),
        "{r:?}"
    );
    assert_eq!(fake.read(&theme).unwrap(), text("'Adwaita'"));
    assert_eq!(fake.read(&unit).unwrap(), FragValue::Absent);
    assert_eq!(fake.read(&scheme).unwrap(), text("'default'"));
}

#[test]
fn backend_failure_rolls_back_files() {
    let (fx, fake) = user_fx();
    let f = fx.cfg("settings.ini");
    fs::write(&f, GTK).unwrap();
    fake.fail_writes.store(true, Ordering::SeqCst);
    let o = fx.app.apply(change(
        "look.gtk",
        vec![
            (ini(&f, "Settings", "gtk-theme-name"), text("N")),
            (dconf("/org/gnome/desktop/interface/gtk-theme"), text("'N'")),
        ],
    ));
    assert_eq!(o, Outcome::Error { kind: "io" });
    assert_eq!(fs::read_to_string(&f).unwrap(), GTK);
    // Tamamen geri dönen işlem iz bırakmaz (depo büyümez).
    assert_eq!(fx.snap_count(), 0);
}

#[test]
fn rollback_that_cannot_undo_is_partial_and_interrupted() {
    let (fx, fake) = user_fx();
    let f = fx.cfg("settings.ini");
    fs::write(&f, GTK).unwrap();
    // Dosya yazıldıktan sonra, dconf yazımı sırasında kullanıcı dosyayı düzenler; dconf da
    // başarısız olur → dosya geri alınamaz (kullanıcının düzenlemesi ezilmez).
    let ff = f.clone();
    *fake.on_write.lock().unwrap() = Some(Box::new(move || {
        fs::write(&ff, "[Settings]\nelle=1\n").unwrap();
    }));
    fake.fail_writes.store(true, Ordering::SeqCst);
    let o = fx.app.apply(change(
        "look.gtk",
        vec![
            (ini(&f, "Settings", "gtk-theme-name"), text("N")),
            (dconf("/org/gnome/desktop/interface/gtk-theme"), text("'N'")),
        ],
    ));
    let Outcome::Partial { id, failed } = o else {
        panic!("{o:?}")
    };
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].kind, "ini_key");
    assert_eq!(fs::read_to_string(&f).unwrap(), "[Settings]\nelle=1\n");
    assert_eq!(fx.app.show(&id).unwrap().unwrap().state, "interrupted");
}

// ---------------------------------------------------------------------------------------------
// İnceleme regresyonları (H3, M1, M2, M4)

/// Depodaki snapshot dizini sayısı (geçerli ya da değil).
fn snap_dirs(fx: &Fx) -> usize {
    fs::read_dir(fx.snapshots())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn repeated_failing_applies_do_not_grow_the_store() {
    let (fx, fake) = user_fx();
    // 1) Ana dosya yok (Line), 2) salt okunur hedef, 3) yazılamayan dizinde yeni dosya,
    // 4) yazım sırasında dış değişiklik (Changed), 5) arka uç hatası.
    let hypr = fx.cfg("hypr/hyprland.conf");
    let ro = fx.cfg("ro.ini");
    fs::write(&ro, "[S]\nk=1\n").unwrap();
    fs::set_permissions(&ro, fs::Permissions::from_mode(0o444)).unwrap();
    let locked_dir = fx.cfg("locked");
    fs::create_dir_all(&locked_dir).unwrap();
    fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o500)).unwrap();
    let racy = fx.cfg("racy.ini");
    fs::write(&racy, "[S]\nk=1\n").unwrap();
    for _ in 0..10 {
        fx.advance(5_000);
        let line = Fragment::Line {
            file: hypr.clone(),
            marker: "source = x".into(),
            at: Position::End,
        };
        assert_eq!(
            fx.app
                .apply(change("hypr.source", vec![(line, text("source = x"))])),
            Outcome::Error { kind: "not_found" }
        );
        assert!(matches!(
            fx.app
                .apply(change("a", vec![(ini(&ro, "S", "k"), text("2"))])),
            Outcome::ExternallyManaged { .. }
        ));
        assert!(matches!(
            fx.app.apply(change(
                "b",
                vec![(ini(&locked_dir.join("new.ini"), "S", "k"), text("2"))]
            )),
            Outcome::ExternallyManaged { .. }
        ));
        let r = racy.clone();
        crate::write::BEFORE_RECHECK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                fs::write(&r, "[S]\nk=elle\n").unwrap();
            }));
        });
        assert_eq!(
            fx.app
                .apply(change("c", vec![(ini(&racy, "S", "k"), text("2"))])),
            Outcome::Changed
        );
        fs::write(&racy, "[S]\nk=1\n").unwrap();
        fake.fail_writes.store(true, Ordering::SeqCst);
        assert_eq!(
            fx.app.apply(change(
                "d",
                vec![(dconf("/org/gnome/desktop/interface/gtk-theme"), text("'x'"))]
            )),
            Outcome::Error { kind: "io" }
        );
        fake.fail_writes.store(false, Ordering::SeqCst);
    }
    assert_eq!(snap_dirs(&fx), 0);
    fs::set_permissions(&locked_dir, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn prune_during_restore_does_not_resurrect_a_blobless_record() {
    let mut fx = Fx::new();
    fx.app.set_limits(1, TOTAL_LIMIT, TOTAL_BYTES_LIMIT);
    let f = fx.cfg("a.ini");
    fs::write(&f, "[S]\nk=0\n").unwrap();
    let k = ini(&f, "S", "k");
    let mut ids = Vec::new();
    for v in ["1", "2", "3"] {
        fx.advance(5_000);
        ids.push(applied(
            fx.app.apply(change("one", vec![(k.clone(), text(v))])),
        ));
    }
    // Ayar başına 1: sabitlenmiş ilk + son kalır.
    fx.advance(5_000);
    let r = fx.app.restore(&ids[2], RestoreMode::Safe);
    assert!(
        matches!(r, RestoreOutcome::Restored { id: Some(_) }),
        "{r:?}"
    );
    // Budanan eski kayıt dirilmedi: kalan her dizinin manifest'i ve blob'ları tam.
    for e in fs::read_dir(fx.snapshots()).unwrap() {
        let e = e.unwrap();
        if !e.file_type().unwrap().is_dir() {
            continue;
        }
        let id = SnapshotId::parse(e.file_name().to_str().unwrap()).unwrap();
        let m = load_manifest(&fx, &id);
        for en in &m.fragments {
            for s in [&en.before, &en.after] {
                if !matches!(s, crate::store::Stored::Absent) {
                    assert!(
                        e.path().join(format!("{}.blob", blob_of(s))).exists(),
                        "{id}: blob yok"
                    );
                }
            }
        }
    }
    assert!(
        !fx.snapshots().join(ids[2].as_str()).exists() || {
            load_manifest(&fx, &ids[2]).state != TxState::Applied
        }
    );
    // Sabitlenmiş ilk hâlâ geri yüklenebilir.
    fx.advance(5_000);
    let r = fx.app.restore(&ids[0], RestoreMode::Force);
    assert!(matches!(r, RestoreOutcome::Restored { .. }), "{r:?}");
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=0\n");
}

/// Her okuma sayılır ve yavaştır.
struct SlowCounting {
    reads: Arc<AtomicU64>,
    delay: Duration,
}

impl crate::Backend for SlowCounting {
    fn read(&self, _: &Fragment) -> Result<FragValue, ApplyError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        Ok(text("'x'"))
    }
    fn write(&self, _: &Fragment, _: &FragValue) -> Result<(), ApplyError> {
        Ok(())
    }
}

#[test]
fn list_reads_each_external_target_once_within_a_budget() {
    let (fx, _) = user_fx();
    // 12 işlem, hepsi aynı dconf anahtarı; 12 işlem, her biri farklı anahtar.
    let shared = dconf("/org/gnome/desktop/interface/gtk-theme");
    for i in 0..12 {
        fx.advance(5_000);
        applied(fx.app.apply(change(
            &format!("s{i}"),
            vec![(shared.clone(), text(&format!("'v{i}'")))],
        )));
    }
    let reads = Arc::new(AtomicU64::new(0));
    let mut env = fx.app.env().clone();
    env.test_instance = false;
    let app = Applier::open(env.clone()).with_backend(Box::new(SlowCounting {
        reads: reads.clone(),
        delay: Duration::from_millis(1),
    }));
    assert_eq!(
        app.list(&Filter {
            setting: None,
            limit: 200
        })
        .unwrap()
        .len(),
        12
    );
    assert_eq!(
        reads.load(Ordering::SeqCst),
        1,
        "aynı dış hedef bir kez okunmalı"
    );

    for i in 0..12 {
        fx.advance(5_000);
        applied(fx.app.apply(change(
            &format!("d{i}"),
            vec![(dconf(&format!("/a/k{i}")), text("'1'"))],
        )));
    }
    let reads = Arc::new(AtomicU64::new(0));
    let slow = Applier::open(env).with_backend(Box::new(SlowCounting {
        reads: reads.clone(),
        delay: Duration::from_millis(400),
    }));
    let start = Instant::now();
    let list = slow
        .list(&Filter {
            setting: None,
            limit: 200,
        })
        .unwrap();
    let took = start.elapsed();
    assert_eq!(list.len(), 24);
    // 13 farklı hedef × 400 ms = 5.2 s olurdu. Bütçe (2 s) okuma SAYISIYLA doğrulanır, duvar
    // saatiyle değil: her okuma ≥ 400 ms uyur, bütçe dolunca yeni okuma başlamaz → en fazla
    // ⌈2 s / 400 ms⌉ = 5 okuma (+1 pay). Süre üst sınırı yük altında (bellek bekçisi test
    // sürecini saniyelerce dondurabiliyor) yanlış düşüyordu; yalnızca tanı için yazılır.
    let n = reads.load(Ordering::SeqCst);
    let max_reads = PEEK_EXT_BUDGET.as_millis().div_ceil(400) as u64 + 1;
    assert!(n <= max_reads, "{n} okuma (en fazla {max_reads}), {took:?}");
}

#[test]
fn failed_final_manifest_save_still_reports_applied() {
    let fx = Fx::new();
    let f = fx.cfg("a.ini");
    fs::write(&f, "[S]\nk=1\n").unwrap();
    // Yazım sırasında snapshot dizinleri salt okunur olur → Applied kaydı yazılamaz.
    let snaps = fx.snapshots();
    crate::write::BEFORE_RECHECK.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || {
            for e in fs::read_dir(&snaps).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() {
                    fs::set_permissions(e.path(), fs::Permissions::from_mode(0o500)).unwrap();
                }
            }
        }));
    });
    let id = applied(
        fx.app
            .apply(change("x", vec![(ini(&f, "S", "k"), text("2"))])),
    );
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=2\n");
    let sdir = fx.snapshots().join(id.as_str());
    fs::set_permissions(&sdir, fs::Permissions::from_mode(0o700)).unwrap();
    // Diskte Pending kalmıştı → Interrupted; yine de geri alınabilir.
    let s = fx.app.show(&id).unwrap().unwrap();
    assert_eq!(s.state, "interrupted");
    fx.advance(10_000);
    let r = fx.app.restore(&id, RestoreMode::Safe);
    assert!(
        matches!(r, RestoreOutcome::Restored { id: Some(_) }),
        "{r:?}"
    );
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\nk=1\n");
}

#[test]
fn for_test_env_never_uses_system_backend() {
    let t = tempfile::tempdir().unwrap();
    let app = Applier::open(Env::for_test(t.path()));
    let d = change(
        "look.gtk4",
        vec![(dconf("/org/gnome/desktop/interface/gtk-theme"), text("'x'"))],
    );
    assert_eq!(
        app.apply(d),
        Outcome::Skipped {
            reason: Skip::TestInstance
        }
    );
}

// --- Parça/manifest sınırları (birleşik görünüm: tek değişiklikte ~110 parça) ---

const UNIFIED_FILES: [&str; 5] = [
    "kdeglobals",
    "gtk-3.0/settings.ini",
    "gtk-4.0/settings.ini",
    "gtk-3.0/gtk.css",
    "qt6ct/qt6ct.conf",
];

/// Birleşik görünümün dosyaları (kullanıcının mevcut içeriğiyle); (yol, içerik) döner.
fn unified_look_files(fx: &Fx) -> Vec<(PathBuf, String)> {
    let mut kde = String::from("[General]\nColorScheme=Breeze\n");
    for g in 0..7 {
        kde.push_str(&format!("\n[Colors:Group{g}]\nBackgroundNormal=1,2,3\n"));
    }
    let contents = [
        kde,
        "[Settings]\ngtk-theme-name=Adwaita\n".into(),
        "[Settings]\ngtk-theme-name=Adwaita\n".into(),
        "/* kullanıcı */\n".into(),
        "[Appearance]\nstyle=Fusion\n".into(),
    ];
    UNIFIED_FILES
        .iter()
        .zip(contents)
        .map(|(rel, t)| {
            let p = fx.cfg(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, &t).unwrap();
            (p, t)
        })
        .collect()
}

/// kdeglobals 7 grup × 12 anahtar + GTK 3/4 settings.ini 2 × 12 + gtk.css `@import` satırı +
/// qt6ct bloğu = 110 parça.
fn unified_look(fx: &Fx, value: &str) -> Change {
    let [kde, gtk3, gtk4, css, qt] = UNIFIED_FILES.map(|r| fx.cfg(r));
    let mut writes = Vec::new();
    for g in 0..7 {
        for k in 0..12 {
            writes.push((
                ini(&kde, &format!("Colors:Group{g}"), &format!("Key{k}")),
                text(&format!("{value},{k}")),
            ));
        }
    }
    for f in [&gtk3, &gtk4] {
        for k in 0..12 {
            writes.push((
                ini(f, "Settings", &format!("gtk-key-{k}")),
                text(&format!("{value}{k}")),
            ));
        }
    }
    writes.push((
        Fragment::Line {
            file: css,
            marker: "@import 'nuxut.css';".into(),
            at: Position::Start,
        },
        text("@import 'nuxut.css';"),
    ));
    writes.push((
        Fragment::Block {
            file: qt,
            id: "palette".into(),
        },
        text(&format!("color_scheme_path={value}\n")),
    ));
    assert_eq!(writes.len(), 110);
    change("look.mode", writes)
}

#[test]
fn unified_look_with_110_fragments_applies_and_restores() {
    let fx = Fx::new();
    let originals = unified_look_files(&fx);
    let id = applied(fx.app.apply(unified_look(&fx, "dark")));
    let kde = fs::read_to_string(&originals[0].0).unwrap();
    assert!(kde.contains("Key11=dark,11"), "{kde}");
    assert!(
        fs::read_to_string(&originals[3].0)
            .unwrap()
            .starts_with("@import 'nuxut.css';\n")
    );
    assert_eq!(fx.app.show(&id).unwrap().unwrap().fragments.len(), 110);
    let size = fs::metadata(fx.snapshots().join(id.as_str()).join("manifest.toml"))
        .unwrap()
        .len();
    eprintln!("110 parçalık manifest: {size} bayt");
    assert!(size * 4 < crate::store::MAX_MANIFEST_BYTES, "{size}");

    // 2 s içinde aynı ayar: birleşir (örtüşen parçalar bir kez sayılır), tek kayıt.
    fx.advance(500);
    assert_eq!(applied(fx.app.apply(unified_look(&fx, "light"))), id);
    assert_eq!(fx.snap_count(), 1);
    assert!(
        fs::read_to_string(&originals[0].0)
            .unwrap()
            .contains("Key11=light,11")
    );

    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    for (p, t) in &originals {
        assert_eq!(&fs::read_to_string(p).unwrap(), t, "{}", p.display());
    }
}

#[test]
fn fragment_count_and_address_budget_are_enforced() {
    let fx = Fx::new();
    let f = fx.cfg("many.ini");
    fs::write(&f, "[S]\n").unwrap();
    let writes = |n: usize, from: usize, v: &str| -> Vec<(Fragment, FragValue)> {
        (from..from + n)
            .map(|i| (ini(&f, "S", &format!("k{i}")), text(v)))
            .collect()
    };
    // MAX_FRAGMENTS + 1: reddedilir, iz yok.
    let o = fx.app.apply(change("x", writes(MAX_FRAGMENTS + 1, 0, "1")));
    assert_eq!(o, Outcome::Error { kind: "invalid" });
    assert_eq!(fs::read_to_string(&f).unwrap(), "[S]\n");
    assert_eq!(snap_dirs(&fx), 0);

    // Adres bütçesi: parça sayısı sınırın altında ama toplam adres > MAX_ADDRESS_BYTES.
    let many: Vec<(Fragment, FragValue)> = (0..80)
        .map(|i| {
            let file = fx.cfg(&format!("{}/{i}.ini", "d".repeat(200)));
            let key = format!("{i:02}{}", "k".repeat(1000));
            (ini(&file, &"s".repeat(256), &key), text("1"))
        })
        .collect();
    assert!(many.iter().map(|(f, _)| f.address_bytes()).sum::<usize>() > MAX_ADDRESS_BYTES);
    assert!(change("x", many[..20].to_vec()).validate().is_ok());
    assert!(matches!(
        change("x", many).validate(),
        Err(ApplyError::Invalid(_))
    ));
    // Çok uzun yol / yolda denetim karakteri: parça geçersiz.
    let long = PathBuf::from(format!("/{}", "p".repeat(MAX_PATH_BYTES)));
    assert!(ini(&long, "S", "k").validate().is_err());
    assert!(ini(Path::new("/a\nb"), "S", "k").validate().is_err());

    // Tam MAX_FRAGMENTS: geçer.
    let id = applied(fx.app.apply(change("x", writes(MAX_FRAGMENTS, 0, "1"))));
    assert_eq!(
        fx.app.show(&id).unwrap().unwrap().fragments.len(),
        MAX_FRAGMENTS
    );
    // Aynı MAX_FRAGMENTS parça 2 s içinde: birleşim yine MAX_FRAGMENTS → birleşir.
    fx.advance(500);
    assert_eq!(
        applied(fx.app.apply(change("x", writes(MAX_FRAGMENTS, 0, "2")))),
        id
    );
    // Yeni bir parça birleşimi sınırın üstüne çıkarır: ayrı işlem olur.
    fx.advance(500);
    let id2 = applied(fx.app.apply(change("x", writes(1, MAX_FRAGMENTS, "1"))));
    assert_ne!(id2, id);
    assert_eq!(
        fx.app.show(&id).unwrap().unwrap().fragments.len(),
        MAX_FRAGMENTS
    );
}

/// Geçerli bir işlemin olabilecek en uzun manifest'i: MAX_FRAGMENTS parça, adres bütçesinin
/// tamamı kaçışlı karakterlerle, en uzun etiket/ayar, her `Stored` dolu (en büyük blob/kip).
fn worst_manifest(fx: &Fx, kind: usize) -> crate::store::Manifest {
    use crate::store::{Entry, Manifest, Stored};
    let id = SnapshotId::parse("20261010T100000Z-ffff").unwrap();
    let mut m = Manifest::new(
        id,
        i64::MAX as u64,
        sid(&"a".repeat(SettingId::MAX_LEN)),
        "𝄞".repeat(MAX_LABEL_CHARS),
        Source::Migration,
    );
    m.created_ms = i64::MAX as u64;
    m.pinned = true;
    m.restores = Some(SnapshotId::parse("20261010T100000Z-eeee").unwrap());
    m.state = TxState::Restored(SnapshotId::parse("20261010T100000Z-dddd").unwrap());
    let per = MAX_ADDRESS_BYTES / MAX_FRAGMENTS;
    let base = fx.root.join("cfg").display().to_string();
    // Kaçışlı dolgu (`"` ve `\` iki bayta çıkar; `'` literal dizgeyi engeller).
    let pad = |n: usize| -> String { "\"\\'".chars().cycle().take(n).collect() };
    let hash = "f".repeat(64);
    let stored = |file: bool| {
        if file {
            Stored::File {
                blob: (1 << 20) - 1,
                mode: 0o777,
                hash: hash.clone(),
            }
        } else {
            Stored::Text {
                blob: (1 << 20) - 1,
                hash: hash.clone(),
            }
        }
    };
    for i in 0..MAX_FRAGMENTS {
        let tag = format!("/{i:03}");
        let path_len = |rest: usize| per - rest - base.len() - tag.len();
        let fragment = match kind {
            0 => {
                // 16 bölüt (bölüt başına tırnak/virgül payı en büyük).
                let segs: Vec<String> = (0..16).map(|_| pad(1)).collect();
                Fragment::TomlKey {
                    file: PathBuf::from(format!("{base}{tag}{}", pad(path_len(16)))),
                    key: KeyPath::new(segs).unwrap(),
                }
            }
            1 => Fragment::IniKey {
                file: PathBuf::from(format!("{base}{tag}{}", pad(path_len(2)))),
                section: pad(1),
                key: "k".into(),
            },
            2 => Fragment::Line {
                file: PathBuf::from(format!("{base}{tag}{}", pad(path_len(1)))),
                marker: "x".into(),
                at: Position::Start,
            },
            _ => Fragment::ManagedFile {
                file: PathBuf::from(format!("{base}{tag}{}", pad(path_len(0)))),
            },
        };
        assert_eq!(fragment.address_bytes(), per);
        fragment.validate().unwrap();
        let file = kind == 3;
        m.fragments.push(Entry {
            fragment,
            before: stored(file),
            after: stored(file),
            file_existed: Some(true),
            prior_after: Some(stored(file)),
            section_created: kind == 1,
            file_inherited: true,
        });
    }
    m
}

#[test]
fn worst_case_manifest_fits_and_oversized_is_refused() {
    let fx = Fx::new();
    one_applied(&fx);
    let store = crate::store::Store::open(fx.app.env()).unwrap();
    for kind in 0..4 {
        let m = worst_manifest(&fx, kind);
        let text = toml::to_string(&m).unwrap();
        let per_entry = (text.len() - 2 * MAX_ADDRESS_BYTES) / MAX_FRAGMENTS;
        eprintln!("tür {kind}: {} bayt, parça başına ~{per_entry}", text.len());
        assert!(
            text.len() as u64 <= crate::store::MAX_MANIFEST_BYTES,
            "tür {kind}: {} > {}",
            text.len(),
            crate::store::MAX_MANIFEST_BYTES
        );
        store.save(&m).unwrap();
        assert_eq!(store.load(&m.id).as_ref(), Some(&m), "tür {kind}");

        // Düşmanca: sınırı bir bayt aşan (geçerli TOML, sonu yorum) manifest okunmaz.
        let p = fx.snapshots().join(m.id.as_str()).join("manifest.toml");
        let mut big = text.into_bytes();
        big.push(b'#');
        big.resize(crate::store::MAX_MANIFEST_BYTES as usize + 1, b'x');
        fs::write(&p, &big).unwrap();
        assert_eq!(store.load(&m.id), None, "tür {kind}");
        assert_eq!(fx.app.show(&m.id).unwrap(), None);
    }
    // MAX_FRAGMENTS + 1 parçalı manifest (boyu sınırda olsa da) okunmaz.
    let mut m = worst_manifest(&fx, 2);
    let mut extra = m.fragments[0].clone();
    extra.fragment = Fragment::Dconf { key: "/a".into() };
    extra.file_existed = None;
    extra.before = crate::store::Stored::Absent;
    extra.after = crate::store::Stored::Absent;
    extra.prior_after = None;
    m.fragments.truncate(MAX_FRAGMENTS - 1);
    m.fragments.push(extra.clone());
    let Fragment::Dconf { key } = &mut extra.fragment else {
        unreachable!()
    };
    key.push('b');
    m.fragments.push(extra);
    assert_eq!(m.fragments.len(), MAX_FRAGMENTS + 1);
    save_manifest(&fx, &m);
    assert_eq!(store.load(&m.id), None);
}

// ---------------------------------------------------------------------------------------------
// Shell takipleri: eksik dosyayı yaratma, Nuxut'un yarattığı INI bölümü, geri yüklenen değerler

fn import_line(file: &Path) -> Fragment {
    Fragment::Line {
        file: file.to_path_buf(),
        marker: "@import url(\"nuxut.css\");".into(),
        at: Position::End,
    }
}

const IMPORT: &str = "@import url(\"nuxut.css\");";

#[test]
fn write_creating_creates_missing_line_file_and_removes_it_on_restore() {
    let fx = Fx::new();
    let css = fx.cfg("gtk-3.0/gtk.css");
    // Ata dizin de yok: 0755 ile yaratılır; dosya yalnızca bizim satırımız.
    let c = Change::new(sid("look.gtk"), "test", Source::Shell)
        .write_creating(import_line(&css), text(IMPORT));
    assert_eq!(c.create_missing, vec![css.clone()]);
    let id = applied(fx.app.apply(c));
    assert_eq!(fs::read_to_string(&css).unwrap(), format!("{IMPORT}\n"));
    assert_eq!(
        fs::metadata(&css).unwrap().permissions().mode() & 0o777,
        0o644
    );
    fx.advance(10_000);
    // Yalnızca bizim satırımız kaldı: geri yüklemede dosya silinir.
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert!(!css.exists());

    // Yaratıldıktan sonra kullanıcı kural ekledi: geri yüklemede yalnızca satırımız gider.
    let id = applied(
        fx.app.apply(
            Change::new(sid("look.gtk"), "test", Source::Shell)
                .write_creating(import_line(&css), text(IMPORT)),
        ),
    );
    let user_rule = "window { color: red; }\n";
    let mut now = fs::read_to_string(&css).unwrap();
    now.push_str(user_rule);
    fs::write(&css, &now).unwrap();
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(fs::read_to_string(&css).unwrap(), user_rule);

    // Var olan kullanıcı dosyası: bayt bayt geri döner, silinmez (boş olsa bile).
    for original in ["", "/* kullanıcı */\n.x { }", "a {}\n"] {
        fs::write(&css, original).unwrap();
        fx.advance(10_000);
        let id = applied(
            fx.app.apply(
                Change::new(sid("look.gtk"), "test", Source::Shell)
                    .write_creating(import_line(&css), text(IMPORT)),
            ),
        );
        fx.advance(10_000);
        assert!(matches!(
            fx.app.restore(&id, RestoreMode::Safe),
            RestoreOutcome::Restored { id: Some(_) }
        ));
        assert_eq!(fs::read_to_string(&css).unwrap(), original, "{original:?}");
    }
}

#[test]
fn write_creating_is_opt_in_and_limited_to_own_line_files() {
    let fx = Fx::new();
    let css = fx.cfg("gtk-4.0/gtk.css");
    // Varsayılan `write`: eksik ana dosya yaratılmaz.
    let o = fx.app.apply(
        Change::new(sid("look.gtk"), "test", Source::Shell).write(import_line(&css), text(IMPORT)),
    );
    assert_eq!(o, Outcome::Error { kind: "not_found" });
    assert!(!css.exists());
    // Yaratma izni istekte olmayan bir dosyaya verilemez; yinelenen yol da geçersiz.
    let mut c =
        Change::new(sid("look.gtk"), "test", Source::Shell).write(import_line(&css), text(IMPORT));
    c.create_missing.push(fx.cfg("hypr/hyprland.conf"));
    assert!(matches!(c.validate(), Err(ApplyError::Invalid(_))));
    let mut c = Change::new(sid("look.gtk"), "test", Source::Shell)
        .write_creating(import_line(&css), text(IMPORT));
    c.create_missing.push(css.clone());
    assert!(matches!(c.validate(), Err(ApplyError::Invalid(_))));
    // INI anahtarı için `write_creating` = `write` (INI zaten yaratılabilir).
    let c = Change::new(sid("x"), "test", Source::Shell)
        .write_creating(ini(&fx.cfg("q.ini"), "S", "k"), text("1"));
    assert!(c.create_missing.is_empty());
    // Silme isteği eksik dosyada iş yapmaz, dosya yaratılmaz.
    let o = fx.app.apply(
        Change::new(sid("look.gtk"), "test", Source::Shell)
            .write_creating(import_line(&css), FragValue::Absent),
    );
    assert_eq!(o, Outcome::Noop);
    assert!(!css.exists());
}

const KDE: &str = "[General]\nColorScheme=Breeze\n\n[KDE]\nSingleClick=false\n";

fn kde_colors(f: &Path, groups: &[&str], v: &str) -> Vec<(Fragment, FragValue)> {
    let mut w = vec![(ini(f, "General", "ColorScheme"), text("Nuxut"))];
    for g in groups {
        w.push((ini(f, g, "BackgroundNormal"), text(v)));
        w.push((ini(f, g, "ForegroundNormal"), text(v)));
    }
    w
}

#[test]
fn nuxut_created_sections_are_removed_bytewise_even_in_the_middle() {
    let fx = Fx::new();
    let f = fx.cfg("kdeglobals");
    let groups = ["Colors:Button", "Colors:View", "Colors:Window"];
    for original in [
        KDE,
        "[General]\r\nColorScheme=Breeze",
        "[General]\nColorScheme=Breeze\n\n",
    ] {
        fs::write(&f, original).unwrap();
        fx.advance(10_000);
        let id = applied(
            fx.app
                .apply(change("look.kde", kde_colors(&f, &groups, "1,2,3"))),
        );
        let now = fs::read_to_string(&f).unwrap();
        assert!(
            now.replace("\r\n", "\n")
                .contains("[Colors:View]\nBackgroundNormal=1,2,3"),
            "{now}"
        );
        let m = load_manifest(&fx, &id);
        let created: Vec<_> = m.fragments.iter().filter(|e| e.section_created).collect();
        assert_eq!(created.len(), groups.len() * 2, "General kullanıcınındı");
        fx.advance(10_000);
        assert!(matches!(
            fx.app.restore(&id, RestoreMode::Safe),
            RestoreOutcome::Restored { id: Some(_) }
        ));
        assert_eq!(fs::read_to_string(&f).unwrap(), original, "{original:?}");
    }

    // İki ayrı ayar: önce yaratılan (ortadaki) bölüm tek başına geri yüklenince kalkar, sonra
    // yaratılan kalır; ikisi de geri yüklenince bayt bayt eski hâl.
    fs::write(&f, KDE).unwrap();
    fx.advance(10_000);
    let a = applied(fx.app.apply(change(
        "look.a",
        vec![(ini(&f, "Colors:Button", "BackgroundNormal"), text("1"))],
    )));
    fx.advance(10_000);
    let b = applied(fx.app.apply(change(
        "look.b",
        vec![(ini(&f, "Colors:Window", "BackgroundNormal"), text("2"))],
    )));
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&a, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(
        fs::read_to_string(&f).unwrap(),
        format!("{KDE}\n[Colors:Window]\nBackgroundNormal=2\n")
    );
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&b, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(fs::read_to_string(&f).unwrap(), KDE);
}

#[test]
fn user_sections_are_never_dropped() {
    let fx = Fx::new();
    let f = fx.cfg("kdeglobals");
    // Kullanıcının ortada boş bir bölümü ve yorumlu bir bölümü var; Nuxut ikisine de anahtar ekler.
    let original = "[Empty]\n\n[Commented]\n# kullanıcı notu\n\n[KDE]\nSingleClick=false\n";
    fs::write(&f, original).unwrap();
    let id = applied(fx.app.apply(change(
        "look.kde",
        vec![
            (ini(&f, "Empty", "k"), text("1")),
            (ini(&f, "Commented", "k"), text("1")),
            (ini(&f, "New", "k"), text("1")),
        ],
    )));
    let m = load_manifest(&fx, &id);
    let created: Vec<_> = m
        .fragments
        .iter()
        .filter(|e| e.section_created)
        .map(|e| e.fragment.clone())
        .collect();
    assert_eq!(created, vec![ini(&f, "New", "k")]);
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(fs::read_to_string(&f).unwrap(), original);

    // Nuxut'un yarattığı bölüme kullanıcı kendi anahtarını ekledi: bölüm kalır.
    fx.advance(10_000);
    let id = applied(fx.app.apply(change(
        "look.kde",
        vec![(ini(&f, "Colors:View", "k"), text("1"))],
    )));
    let now = fs::read_to_string(&f)
        .unwrap()
        .replace("[Colors:View]\nk=1\n", "[Colors:View]\nk=1\nmine=2\n");
    fs::write(&f, &now).unwrap();
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(
        fs::read_to_string(&f).unwrap(),
        format!("{original}\n[Colors:View]\nmine=2\n")
    );
}

#[test]
fn manifest_without_section_created_still_loads() {
    let fx = Fx::new();
    let f = fx.cfg("kdeglobals");
    fs::write(&f, KDE).unwrap();
    // Bölüm yaratmayan işlem: alan yazılmaz (eski sürümlerle bayt düzeyinde aynı biçim).
    let (plain, _) = one_applied(&fx);
    let raw =
        fs::read_to_string(fx.snapshots().join(plain.as_str()).join("manifest.toml")).unwrap();
    assert!(!raw.contains("section_created"), "{raw}");
    // Bölüm yaratan işlem: alan yazılır; eski sürümün kaydı (alan yok) yine okunur ve geri yüklenir.
    let id = applied(fx.app.apply(change(
        "look.kde",
        vec![(ini(&f, "Colors:View", "k"), text("1"))],
    )));
    let p = fx.snapshots().join(id.as_str()).join("manifest.toml");
    let raw = fs::read_to_string(&p).unwrap();
    assert!(raw.contains("section_created = true"), "{raw}");
    fs::write(&p, raw.replace("section_created = true\n", "")).unwrap();
    assert!(!load_manifest(&fx, &id).fragments[0].section_created);
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    // Eski kayıtta bilgi yok: son bölüm (ini::set kuralı) yine kalkar.
    assert_eq!(fs::read_to_string(&f).unwrap(), KDE);
    // `section_created` yalnızca bölümlü INI anahtarında geçerli; başka parçada kayıt bozuk sayılır.
    let mut m = load_manifest(&fx, &plain);
    m.fragments[0].fragment = Fragment::Dconf { key: "/a/b".into() };
    m.fragments[0].file_existed = None;
    m.fragments[0].section_created = true;
    save_manifest(&fx, &m);
    assert_eq!(fx.app.show(&plain).unwrap(), None);
}

#[test]
fn restore_with_values_reports_what_was_written() {
    let fx = Fx::new();
    let f = fx.cfg("gtk-3.0/settings.ini");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(&f, "[Settings]\ngtk-cursor-theme-name=Adwaita\n").unwrap();
    let name = ini(&f, "Settings", "gtk-cursor-theme-name");
    let size = ini(&f, "Settings", "gtk-cursor-theme-size");
    let css = fx.cfg("gtk-3.0/gtk.css");
    let id = applied(
        fx.app.apply(
            Change::new(sid("look.cursor"), "test", Source::Shell)
                .write(name.clone(), text("Bibata"))
                .write(size.clone(), text("32"))
                .write_creating(import_line(&css), text(IMPORT)),
        ),
    );
    fx.advance(10_000);
    let (out, values) = fx.app.restore_with_values(&id, RestoreMode::Safe);
    assert!(matches!(out, RestoreOutcome::Restored { id: Some(_) }));
    assert_eq!(values.setting, Some(sid("look.cursor")));
    assert_eq!(values.get(&name), Some(Some("Adwaita")));
    assert_eq!(values.get(&size), Some(None), "boyut yoktu: kaldırıldı");
    assert_eq!(values.get(&import_line(&css)), Some(None));
    assert_eq!(values.get(&ini(&f, "Settings", "x")), None, "yazılmadı");
    // Debug değer ve yol göstermez.
    let dbg = format!("{values:?}");
    assert!(
        !dbg.contains("Adwaita") && !dbg.contains("settings.ini"),
        "{dbg}"
    );

    // İkinci geri yükleme (yeni işlem) yine geri alınabilir; çakışan parça listede yok.
    fx.advance(1_000);
    let id = applied(
        fx.app
            .apply(change("look.cursor", vec![(name.clone(), text("Bibata"))])),
    );
    let id2 = applied(
        fx.app
            .apply(change("look.cursor", vec![(size.clone(), text("40"))])),
    );
    assert_eq!(id, id2, "2 sn içinde birleşir");
    fs::write(
        &f,
        fs::read_to_string(&f)
            .unwrap()
            .replace("gtk-cursor-theme-size=40", "gtk-cursor-theme-size=48"),
    )
    .unwrap();
    fx.advance(10_000);
    let (out, values) = fx.app.restore_with_values(&id, RestoreMode::Safe);
    assert!(matches!(out, RestoreOutcome::Partial { .. }), "{out:?}");
    assert_eq!(values.get(&name), Some(Some("Adwaita")));
    assert_eq!(values.get(&size), None);
    // Değer yok: bulunamayan kayıt, iş yapılmayan geri yükleme.
    let missing = SnapshotId::parse("20200101T000000Z-0000").unwrap();
    let (out, values) = fx.app.restore_with_values(&missing, RestoreMode::Safe);
    assert_eq!(out, RestoreOutcome::NotFound);
    assert_eq!(values, RestoredValues::default());
}

#[test]
fn apply_derived_merges_into_the_last_untouched_transaction() {
    let fx = Fx::new();
    let f = fx.cfg("kdeglobals");
    fs::write(&f, KDE).unwrap();
    let k = ini(&f, "Colors:View", "BackgroundNormal");
    let id = applied(
        fx.app
            .apply(change("look.kde", vec![(k.clone(), text("1"))])),
    );
    // Süre penceresinin çok ötesinde: yine aynı işleme birleşir (ilk `before` korunur).
    fx.advance(3_600_000);
    assert_eq!(
        applied(
            fx.app
                .apply_derived(change("look.kde", vec![(k.clone(), text("2"))]))
        ),
        id
    );
    assert_eq!(fx.snap_count(), 1);
    // Araya kullanıcı girdi: yeni snapshot (elle değişiklik geri alınabilir kalır).
    let now = fs::read_to_string(&f)
        .unwrap()
        .replace("BackgroundNormal=2", "BackgroundNormal=9");
    fs::write(&f, now).unwrap();
    fx.advance(10_000);
    let id2 = applied(
        fx.app
            .apply_derived(change("look.kde", vec![(k.clone(), text("3"))])),
    );
    assert_ne!(id2, id);
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id2, RestoreMode::Safe),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert!(
        fs::read_to_string(&f)
            .unwrap()
            .contains("BackgroundNormal=9")
    );
    fx.advance(10_000);
    assert!(matches!(
        fx.app.restore(&id, RestoreMode::Force),
        RestoreOutcome::Restored { id: Some(_) }
    ));
    assert_eq!(fs::read_to_string(&f).unwrap(), KDE);
}

/// `list`/`show` satırı oluşturulma anını ms olarak taşır (göreli zaman için; `at` saniyelik).
#[test]
fn summary_carries_created_ms() {
    let fx = Fx::new();
    let f = fx.cfg("settings.ini");
    fs::write(&f, GTK).unwrap();
    fx.advance(1_234);
    let id = applied(fx.app.apply(change(
        "look.gtk",
        vec![(ini(&f, "Settings", "gtk-theme-name"), text("Nuxut"))],
    )));
    let created = 1_791_633_601_234;
    let list = fx.app.list(&Filter::default()).unwrap();
    assert_eq!(list[0].created_ms, created);
    assert_eq!(fx.app.show(&id).unwrap().unwrap().created_ms, created);
    assert_eq!(list[0].at, "2026-10-10T12:00:01Z");
}

// ---------------------------------------------------------------------------------------------
// Parça parça yazılan, Nuxut'un yarattığı dosya (yaratıcı bilgisinin devri)

const LOOK_KEYS: [(&str, &str, &str); 4] = [
    ("look.mode", "gtk-application-prefer-dark-theme", "1"),
    ("look.gtk", "gtk-theme-name", "Nuxut"),
    ("look.icons", "gtk-icon-theme-name", "Papirus"),
    ("look.cursor", "gtk-cursor-theme-name", "Bibata"),
];

/// Dört ayar aynı `[Settings]` bölümüne birer anahtar yazar (ayrı işlemler, ileri sırada).
fn four_look_ops(fx: &Fx, f: &Path) -> Vec<SnapshotId> {
    LOOK_KEYS
        .iter()
        .map(|(setting, key, value)| {
            let id = applied(
                fx.app
                    .apply(change(setting, vec![(ini(f, "Settings", key), text(value))])),
            );
            fx.advance(10_000);
            id
        })
        .collect()
}

fn restore_ok(fx: &Fx, id: &SnapshotId) {
    let r = fx.app.restore(id, RestoreMode::Safe);
    assert!(matches!(r, RestoreOutcome::Restored { id: Some(_) }), "{r:?}");
    fx.advance(10_000);
}

#[test]
fn shared_created_file_forward_restore_removes_file() {
    let fx = Fx::new();
    let f = fx.cfg("gtk-4.0/settings.ini");
    let ids = four_look_ops(&fx, &f);
    let m: Vec<_> = ids.iter().map(|id| load_manifest(&fx, id)).collect();
    assert_eq!(m[0].fragments[0].file_existed, Some(false));
    assert!(!m[0].fragments[0].file_inherited);
    for later in &m[1..] {
        assert_eq!(later.fragments[0].file_existed, Some(true));
        assert!(later.fragments[0].file_inherited);
    }
    for (i, id) in ids.iter().enumerate() {
        restore_ok(&fx, id);
        if i < 3 {
            let now = fs::read_to_string(&f).unwrap();
            assert!(now.contains(LOOK_KEYS[3].1), "{now}");
        }
    }
    assert!(!f.exists());
}

#[test]
fn shared_created_file_lifo_restore_removes_file() {
    let fx = Fx::new();
    let f = fx.cfg("gtk-4.0/settings.ini");
    let ids = four_look_ops(&fx, &f);
    for id in ids.iter().rev() {
        restore_ok(&fx, id);
    }
    assert!(!f.exists());
}

#[test]
fn shared_created_file_any_order_removes_file() {
    let fx = Fx::new();
    let f = fx.cfg("gtk-4.0/settings.ini");
    let ids = four_look_ops(&fx, &f);
    for i in [2, 0, 3, 1] {
        restore_ok(&fx, &ids[i]);
    }
    assert!(!f.exists());
}

#[test]
fn shared_created_file_single_middle_restore_keeps_others() {
    let fx = Fx::new();
    let f = fx.cfg("gtk-4.0/settings.ini");
    let ids = four_look_ops(&fx, &f);
    restore_ok(&fx, &ids[1]);
    let now = fs::read_to_string(&f).unwrap();
    assert_eq!(
        now,
        "[Settings]\ngtk-application-prefer-dark-theme=1\ngtk-icon-theme-name=Papirus\n\
         gtk-cursor-theme-name=Bibata\n"
    );
    // Sonradan aynı ayar yeniden uygulanır: hâlâ uygulanmış yaratıcılardan devralır.
    let again = applied(fx.app.apply(change(
        "look.gtk",
        vec![(ini(&f, "Settings", "gtk-theme-name"), text("Nuxut"))],
    )));
    fx.advance(10_000);
    assert!(load_manifest(&fx, &again).fragments[0].file_inherited);
    for id in [&ids[0], &ids[3], &again, &ids[2]] {
        restore_ok(&fx, id);
    }
    assert!(!f.exists());
}

#[test]
fn preexisting_file_is_never_removed() {
    for pre in ["", "[Settings]\n"] {
        for order in [[0, 1, 2, 3], [3, 2, 1, 0], [1, 3, 0, 2]] {
            let fx = Fx::new();
            let f = fx.cfg("gtk-4.0/settings.ini");
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(&f, pre).unwrap();
            let ids = four_look_ops(&fx, &f);
            for id in &ids {
                let e = &load_manifest(&fx, id).fragments[0];
                assert_eq!(e.file_existed, Some(true));
                assert!(!e.file_inherited);
            }
            for i in order {
                restore_ok(&fx, &ids[i]);
            }
            assert!(f.exists(), "pre={pre:?} order={order:?}");
        }
    }
}

#[test]
fn user_key_in_shared_created_file_keeps_it() {
    for order in [[0, 1, 2, 3], [3, 2, 1, 0]] {
        let fx = Fx::new();
        let f = fx.cfg("gtk-4.0/settings.ini");
        let ids = four_look_ops(&fx, &f);
        // Kullanıcı arada kendi anahtarını (başka bölümde) ve bir yorum ekler.
        let mut now = fs::read_to_string(&f).unwrap();
        now.push_str("\n[Kendi]\nbenim=1\n");
        fs::write(&f, &now).unwrap();
        for i in order {
            restore_ok(&fx, &ids[i]);
        }
        assert_eq!(fs::read_to_string(&f).unwrap(), "[Kendi]\nbenim=1\n");
    }
    // Yalnızca yorum da kullanıcı içeriğidir.
    let fx = Fx::new();
    let f = fx.cfg("gtk-4.0/settings.ini");
    let ids = four_look_ops(&fx, &f);
    let now = fs::read_to_string(&f).unwrap();
    fs::write(&f, format!("# benim notum\n{now}")).unwrap();
    for id in &ids {
        restore_ok(&fx, id);
    }
    assert!(f.exists());
}

#[test]
fn recreated_user_file_does_not_inherit() {
    let fx = Fx::new();
    let f = fx.cfg("gtk-4.0/settings.ini");
    let first = applied(fx.app.apply(change(
        "look.mode",
        vec![(ini(&f, "Settings", "gtk-application-prefer-dark-theme"), text("1"))],
    )));
    fx.advance(10_000);
    // Kullanıcı Nuxut'un dosyasını silip kendi dosyasını yaratır (yaratıcının değeri yok).
    fs::write(&f, "[Settings]\n").unwrap();
    let second = applied(fx.app.apply(change(
        "look.gtk",
        vec![(ini(&f, "Settings", "gtk-theme-name"), text("Nuxut"))],
    )));
    fx.advance(10_000);
    assert!(!load_manifest(&fx, &second).fragments[0].file_inherited);
    restore_ok(&fx, &second);
    // Dosya kullanıcınındır: boş kalsa da silinmez. (Boş `[Settings]` başlığı ise ilk işlemin
    // `section_created` kaydı yüzünden kalkar — bölüm bilgisinin bilinen, ayrı sınırı.)
    assert!(f.exists());
    // İlk işlemin parçası zaten öncesinde (yok): geri yükleme bir şey yazmaz, dosya kalır.
    let r = fx.app.restore(&first, RestoreMode::Safe);
    assert!(matches!(r, RestoreOutcome::Restored { id: None }), "{r:?}");
    assert!(f.exists());
}

#[test]
fn manifest_without_file_inherited_still_loads() {
    let fx = Fx::new();
    let f = fx.cfg("gtk-4.0/settings.ini");
    let ids = four_look_ops(&fx, &f);
    // Yaratan işlemin kaydında alan yazılmaz (eski biçimle aynı); devralanınkinde yazılır.
    let raw0 = fs::read_to_string(fx.snapshots().join(ids[0].as_str()).join("manifest.toml"))
        .unwrap();
    assert!(!raw0.contains("file_inherited"), "{raw0}");
    for id in &ids[1..] {
        let p = fx.snapshots().join(id.as_str()).join("manifest.toml");
        let raw = fs::read_to_string(&p).unwrap();
        assert!(raw.contains("file_inherited = true"), "{raw}");
        // Eski sürümün kaydı: alan yok → okunur, `false` (önceki davranış).
        fs::write(&p, raw.replace("file_inherited = true\n", "")).unwrap();
        assert!(!load_manifest(&fx, id).fragments[0].file_inherited);
        assert!(fx.app.show(id).unwrap().is_some());
    }
    // Eski kayıtlarla ileri sıra: yaratıcı ilk geri alınınca bilgi kaybolur, dosya kalır
    // (önceki davranış); bozulma yok.
    for id in &ids {
        restore_ok(&fx, id);
    }
    assert!(f.exists());
    // `file_inherited` yalnızca var olan dosyanın parçasında geçerli; aksi kayıt bozuk sayılır.
    let mut m = load_manifest(&fx, &ids[0]);
    m.fragments[0].file_inherited = true;
    save_manifest(&fx, &m);
    assert_eq!(fx.app.show(&ids[0]).unwrap(), None);
}

/// Nuxut'un yarattığı dizinler (2026-10-16): yalnızca kendi yarattığı ve boş kalanlar geri
/// yüklemede silinir; kullanıcının var olan dizinine ve içine kullanıcı dosyası konmuş Nuxut
/// dizinine dokunulmaz.
#[test]
fn created_dirs_are_removed_only_when_ours_and_empty() {
    let fx = Fx::new();
    let css = |s: &str| FragValue::File { bytes: format!("/* Nuxut — {s} */\n").into_bytes(), mode: 0 };
    let deep = fx.cfg("tema/a/b/x.css");
    let user_dir = fx.cfg("kullanici");
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(user_dir.join("benim.txt"), "x").unwrap();
    let under_user = fx.cfg("kullanici/yeni/y.css");
    let shared = fx.cfg("paylasilan/z.css");
    let id = applied(fx.app.apply(change(
        "look.mode",
        vec![
            (Fragment::ManagedFile { file: deep.clone() }, css("x")),
            (Fragment::ManagedFile { file: under_user.clone() }, css("y")),
            (Fragment::ManagedFile { file: shared.clone() }, css("z")),
        ],
    )));
    let m = load_manifest(&fx, &id);
    let canon = |p: PathBuf| p.canonicalize().unwrap();
    for d in [fx.cfg("tema"), fx.cfg("tema/a"), fx.cfg("tema/a/b"), fx.cfg("kullanici/yeni"), fx.cfg("paylasilan")] {
        assert!(m.created_dirs.contains(&canon(d.clone())), "{d:?}: {:?}", m.created_dirs);
    }
    assert!(!m.created_dirs.contains(&canon(user_dir.clone())), "kullanıcının dizini kaydedilmez");
    // Kullanıcı Nuxut'un dizinine kendi dosyasını koyar.
    fs::write(fx.cfg("paylasilan/benim.css"), "k").unwrap();
    fx.advance(10_000);
    restore_ok(&fx, &id);
    assert!(!fx.cfg("tema").exists(), "boş kalan Nuxut dizinleri silindi");
    assert!(!fx.cfg("kullanici/yeni").exists());
    assert_eq!(fs::read_to_string(user_dir.join("benim.txt")).unwrap(), "x");
    assert_eq!(fs::read_to_string(fx.cfg("paylasilan/benim.css")).unwrap(), "k", "dolu dizin kalır");
    assert!(!shared.exists());
}

/// Aynı Nuxut dizinine iki ayar yazar (`kitty/` look.mode ve look.motion): ikinci ayar dizini
/// devralır; hangi sırayla geri alınırsa alınsın son boşaltan siler.
#[test]
fn created_dirs_are_inherited_across_settings() {
    for reverse in [false, true] {
        let fx = Fx::new();
        let file = |n: &str| FragValue::File { bytes: format!("# Nuxut — {n}\n").into_bytes(), mode: 0 };
        let a = applied(fx.app.apply(change("look.mode", vec![(Fragment::ManagedFile { file: fx.cfg("kitty/bir.conf") }, file("bir"))])));
        fx.advance(10_000);
        let b = applied(fx.app.apply(change("look.motion", vec![(Fragment::ManagedFile { file: fx.cfg("kitty/iki.conf") }, file("iki"))])));
        let dir = fx.cfg("kitty").canonicalize().unwrap();
        assert!(load_manifest(&fx, &b).created_dirs.contains(&dir), "devralındı");
        fx.advance(10_000);
        let (first, second) = if reverse { (b, a) } else { (a, b) };
        restore_ok(&fx, &first);
        assert!(fx.cfg("kitty").exists(), "öbür ayarın dosyası duruyor");
        restore_ok(&fx, &second);
        assert!(!fx.cfg("kitty").exists(), "son boşaltan sildi (ters: {reverse})");
    }
}

/// Bozuk kayıttaki göreli ya da `..` içeren dizin yolu: kayıt yok sayılır (hiçbir şey silinmez).
#[test]
fn created_dirs_in_manifest_are_validated() {
    let fx = Fx::new();
    let id = applied(fx.app.apply(change("look.mode", vec![(Fragment::ManagedFile { file: fx.cfg("d/x.css") }, FragValue::File { bytes: "/* Nuxut — x */\n".as_bytes().to_vec(), mode: 0 })])));
    let mut m = load_manifest(&fx, &id);
    m.created_dirs.push(PathBuf::from("goreli"));
    let store = crate::store::Store::open(fx.app.env()).unwrap();
    save_manifest(&fx, &m);
    assert!(store.load(&id).is_none(), "göreli yol: kayıt reddedilir");
    m.created_dirs.pop();
    m.created_dirs.push(fx.cfg("d/../e"));
    save_manifest(&fx, &m);
    assert!(store.load(&id).is_none(), "`..`: kayıt reddedilir");
}
