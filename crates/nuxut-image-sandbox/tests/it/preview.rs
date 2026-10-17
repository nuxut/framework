//! Tema önizlemeleri + gerçek işçi (ADR-0007): kullanıcı dizinindeki (kök sahipli olmayan)
//! SVG ikon teması ve yalnızca-SVG hyprcursor şekli yalıtılmış işçide çözülür; işçisiz yer
//! tutucu; işçi kullanılamıyorken SVG süreç içine düşmez (K2-b).

use std::path::Path;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use nuxut_image::preview::{PreviewEntry, cursor_request, icon_request, outcome};
use nuxut_image::{CursorRole, CursorTheme, IconRole, IconTheme, ImageError, LoadRequest, Loader, PreviewKey, SandboxFailure};
use nuxut_image_sandbox::{Sandbox, SandboxConfig};

use crate::{WORKER, serial};

fn cfg() -> SandboxConfig {
    SandboxConfig {
        worker: Some(WORKER.into()),
        timeout: Duration::from_secs(10),
        spawn_timeout: Duration::from_secs(10),
        ..SandboxConfig::default()
    }
}

const RED_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#ff0000"/></svg>"##;

/// `<root>/<id>/scalable/apps/<ad>.svg` biçiminde sahte ölçeklenebilir ikon teması.
fn svg_icon_theme(root: &Path, id: &str, names: &[&str]) {
    let dir = root.join(id).join("scalable/apps");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        root.join(id).join("index.theme"),
        format!("[Icon Theme]\nName={id}\nDirectories=scalable/apps\n[scalable/apps]\nSize=48\nType=Scalable\nMinSize=8\nMaxSize=512\n"),
    )
    .unwrap();
    for n in names {
        std::fs::write(dir.join(format!("{n}.svg")), RED_SVG).unwrap();
    }
}

/// Önizleme akışı: anahtarlı iş, sonuç `outcome` ile önbellek kaydına.
fn run(loader: &mut Loader, key: PreviewKey, req: LoadRequest) -> Option<PreviewEntry> {
    let (tx, rx) = mpsc::channel();
    loader.load_keyed(key.load_key(), req, move |r| tx.send(outcome(r)).unwrap());
    rx.recv_timeout(Duration::from_secs(30)).unwrap()
}

fn icon_key(theme: &str, role: IconRole) -> PreviewKey {
    PreviewKey::Icon { theme: theme.into(), role, px: 32 }
}

#[test]
fn user_dir_svg_icon_theme_preview_resolves_in_the_sandbox() {
    let _s = serial();
    let root = tempfile::tempdir().unwrap();
    svg_icon_theme(root.path(), "Sahte", &["folder", "utilities-terminal"]);
    let theme = Arc::new(IconTheme::with_dirs("Sahte", vec![root.path().to_owned()], Vec::new()));

    // Yalıtımsız: bugünkü davranış, "yok" (yer tutucu).
    let mut plain = Loader::new(1);
    let got = run(&mut plain, icon_key("Sahte", IconRole::Folder), icon_request(theme.clone(), IconRole::Folder, 32, 1));
    assert!(matches!(got, Some(None)), "işçisiz SVG yer tutucu");

    // Yalıtımlı: işçide çözülür, tek kare, hedef boyutta, kırmızı.
    let sb = Arc::new(Sandbox::new(cfg()));
    let mut loader = Loader::new(1).with_sandbox(sb.clone());
    for role in [IconRole::Folder, IconRole::Terminal] {
        let got = run(&mut loader, icon_key("Sahte", role), icon_request(theme.clone(), role, 16, 2));
        let img = got.flatten().expect("işçide çözüldü");
        assert_eq!(img.frames.len(), 1);
        assert_eq!(img.first().size(), (32, 32));
        assert_eq!(&img.first().pixels()[..4], &[255, 0, 0, 255]);
    }
    assert_eq!(sb.stats().decoded, 2, "iki SVG işçide");
    assert_eq!(sb.stats().in_process_fallbacks, 0);
    // Temada olmayan rol işçiye gitmeden "yok".
    let got = run(&mut loader, icon_key("Sahte", IconRole::Trash), icon_request(theme, IconRole::Trash, 16, 2));
    assert!(matches!(got, Some(None)));
    assert_eq!(sb.stats().decoded, 2);
}

#[test]
fn unavailable_sandbox_never_decodes_svg_in_process() {
    let _s = serial();
    let root = tempfile::tempdir().unwrap();
    svg_icon_theme(root.path(), "Sahte", &["folder"]);
    let theme = Arc::new(IconTheme::with_dirs("Sahte", vec![root.path().to_owned()], Vec::new()));
    let sb = Arc::new(Sandbox::new(SandboxConfig { worker: Some(root.path().join("yok-isci")), ..cfg() }));
    assert_eq!(sb.isolation(), nuxut_image::Isolation::Unavailable);
    let mut loader = Loader::new(1).with_sandbox(sb.clone());
    let (tx, rx) = mpsc::channel();
    loader.load(icon_request(theme, IconRole::Folder, 32, 1), move |r| tx.send(r.map(|d| d.size())).unwrap());
    let r = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(matches!(r, Err(ImageError::Sandbox(SandboxFailure::Unavailable))), "{r:?}");
    assert_eq!(sb.stats().in_process_fallbacks, 0, "SVG süreç içine düşmedi");
    assert_eq!(sb.stats().spawned, 0);
}

/// Sıkıştırmasız ZIP (hyprcursor `.hlc`): yerel başlıklar + merkezi dizin + EOCD.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
            }
        }
        !c
    }
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let offset = out.len() as u32;
        let (crc, len, nlen) = (crc32(data), data.len() as u32, name.len() as u16);
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for v in [crc, len, len] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        for v in [crc, len, len] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        central.extend_from_slice(&nlen.to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let (cd_offset, cd_len, n) = (out.len() as u32, central.len() as u32, files.len() as u16);
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[test]
fn user_dir_svg_hyprcursor_shape_resolves_in_the_sandbox() {
    let _s = serial();
    let root = tempfile::tempdir().unwrap();
    let t = root.path().join("SahteImlec");
    std::fs::create_dir_all(t.join("hyprcursors")).unwrap();
    std::fs::write(t.join("manifest.hl"), "name = SahteImlec\ncursors_directory = hyprcursors\n").unwrap();
    let meta = "hotspot_x = 0.0\nhotspot_y = 0.0\ndefine_override = left_ptr;arrow\ndefine_size = 0, a.svg\n";
    let hlc = stored_zip(&[("meta.hl", meta.as_bytes()), ("a.svg", RED_SVG.as_bytes())]);
    std::fs::write(t.join("hyprcursors/default.hlc"), hlc).unwrap();
    let theme = Arc::new(CursorTheme::with_dirs("SahteImlec", vec![root.path().to_owned()]));
    let key = || PreviewKey::Cursor { theme: "SahteImlec".into(), role: CursorRole::Default, px: 32 };

    let mut plain = Loader::new(1);
    assert!(matches!(run(&mut plain, key(), cursor_request(theme.clone(), CursorRole::Default, 32, true)), Some(None)));

    let sb = Arc::new(Sandbox::new(cfg()));
    let mut loader = Loader::new(1).with_sandbox(sb.clone());
    let img = run(&mut loader, key(), cursor_request(theme, CursorRole::Default, 32, true)).flatten().expect("işçide çözüldü");
    assert_eq!((img.frames.len(), img.first().size()), (1, (32, 32)), "güvenilmeyende tek kare");
    assert_eq!(&img.first().pixels()[..4], &[255, 0, 0, 255]);
    assert_eq!(sb.stats().decoded, 1);
}
