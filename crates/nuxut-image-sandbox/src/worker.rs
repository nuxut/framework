//! İşçi süreci (ADR-0007 §İşçinin kendini kısıtlaması). `worker_main` girdi okumadan önce
//! kendini kilitler, el sıkışmayı yazar ve stdin'den istekleri sırayla çözer. Boştayken
//! `read(0)`'da bloklanır: CPU harcamaz; stdin kapanınca (EOF) çıkar.
//!
//! Profil (`--profile=icon|wallpaper`, yoksa `icon`; [`WorkerProfile`]) sınırları belirler ve
//! el sıkışmada beyan edilir. Kilitleme sırası (biri başarısız olursa çıkış kodu
//! [`EXIT_LOCKDOWN`]):
//! 0. Profil önceliği (en iyi çaba): `nice` (duvar kağıdı 10) ve `/proc/self/oom_score_adj`
//!    (duvar kağıdı 1000: bellek sıkışınca çekirdek önce işçiyi öldürür, shell'i değil),
//! 1. `setrlimit`: AS (ikon 768 MiB, duvar kağıdı 2.5 GiB), CPU (4/5 sn, 10/12 sn), FSIZE 0,
//!    NPROC 0, CORE 0, MEMLOCK 0,
//! 2. `PR_SET_NO_NEW_PRIVS`, `PR_SET_DUMPABLE 0`,
//! 3. Landlock: dosya sistemi + TCP + soyut soket/sinyal kapsamı, kuralsız (hiç erişim yok;
//!    eski ABI'de kısmi uygulama kabul; hiç uygulanamazsa çıkış 70),
//! 4. `NOFILE` 3 (landlock kural kümesi fd'si kapandıktan sonra),
//! 5. seccomp-bpf, varsayılan `KILL_PROCESS`, izin listesi [`allowed_syscalls`].
//!
//! `PR_SET_PDEATHSIG` kullanılmaz: işçiyi başlatan *thread* bitince tetiklenir; havuzdaki işçi
//! başka thread'den kullanılırken ölürdü. Ebeveyn ölünce stdin EOF olur (boştaki işçi çıkar);
//! çözüm ortasındaysa `RLIMIT_CPU` ile biter.
//!
//! Miras fd'ler: rustix'te `close_range` yok ve fd kapatmak `unsafe` ister (bu crate'te yok).
//! Yerine seccomp onları etkisiz kılar: `read` yalnızca fd 0, `write` yalnızca fd 1, `mmap`
//! yalnızca anonim (fd = -1); fd alan başka çağrı (`close`, `dup`, `fcntl`, `ioctl`, `sendmsg`…)
//! listede yok. Sayıları (yalnızca sayı) el sıkışmada bildirilir; ebeveyn debug log'a yazar.

use std::io::{Read, Write};

use nuxut_image::{DecodeOptions, Limits, animation_info, decode_isolated, is_animated};

use crate::protocol::{self, ErrorCode, REQUEST_HEADER_LEN, Request, Response};
use crate::{EXIT_LOCKDOWN, EXIT_PROTOCOL, WorkerProfile};

#[derive(Debug, Default, Clone, Copy)]
struct Flags {
    profile: WorkerProfile,
    /// Test: seccomp kurulmaz (landlock katmanını tek başına sınamak için).
    no_seccomp: bool,
    /// Test: yanlış el sıkışma.
    bad_handshake: bool,
    /// Test: el sıkışmada başka profil beyan et.
    wrong_profile: bool,
}

fn parse_flags() -> Flags {
    let mut f = Flags::default();
    for a in std::env::args().skip(1) {
        if let Some(p) = WorkerProfile::from_arg(&a) {
            f.profile = p;
            continue;
        }
        #[cfg(feature = "sandbox-test-hooks")]
        match a.as_str() {
            "--test-no-seccomp" => f.no_seccomp = true,
            "--test-bad-handshake" => f.bad_handshake = true,
            "--test-wrong-profile" => f.wrong_profile = true,
            _ => {}
        }
    }
    f
}

/// Yardımcı ikilinin tek satırı: `fn main() { nuxut_image_sandbox::worker_main() }`.
pub fn worker_main() -> ! {
    // Panik iletisi stderr'e yazılamaz (seccomp `write`'ı yalnızca fd 1'e izin verir); sessiz
    // kanca ile panik doğrudan süreç sonuna gider, ebeveyn `Killed` görür.
    std::panic::set_hook(Box::new(|_| {}));
    let flags = parse_flags();
    #[cfg(feature = "sandbox-test-hooks")]
    crate::hooks::remember_pid();
    let Ok(mut handshake) = lockdown(flags) else { std::process::exit(EXIT_LOCKDOWN) };
    // Kilit kuruldu: yalnızca bundan sonra güvenilmeyen bayt ayrıştırılabilir (bulgu 2c).
    nuxut_image::enter_worker_mode();
    if flags.wrong_profile {
        handshake.profile = WorkerProfile::ALL.into_iter().find(|p| *p != flags.profile).unwrap_or_default().code();
    }
    let mut stdout = std::io::stdout().lock();
    let mut hs = handshake.encode();
    if flags.bad_handshake {
        hs[0] ^= 0xFF;
    }
    if stdout.write_all(&hs).and_then(|()| stdout.flush()).is_err() {
        std::process::exit(0);
    }
    let mut stdin = std::io::stdin().lock();
    serve(flags.profile, &mut stdin, &mut stdout);
    std::process::exit(0);
}

fn serve(profile: WorkerProfile, stdin: &mut impl Read, stdout: &mut impl Write) {
    let caps = profile.caps();
    loop {
        let mut head = [0u8; REQUEST_HEADER_LEN];
        if stdin.read_exact(&mut head).is_err() {
            return; // EOF: ebeveyn işçiyi kapattı.
        }
        let Some(req) = Request::decode(&head, caps.max_input_bytes) else { std::process::exit(EXIT_PROTOCOL) };
        #[cfg(feature = "sandbox-test-hooks")]
        if req.hook == crate::hooks::Hook::SpinBeforePayload as u8 {
            crate::hooks::spin();
        }
        let mut payload = vec![0u8; req.payload_len as usize];
        if stdin.read_exact(&mut payload).is_err() {
            return;
        }
        let (resp, pixels) = handle(profile, &req, &payload);
        drop(payload);
        let ok = stdout.write_all(&resp.encode()).and_then(|()| stdout.write_all(&pixels)).and_then(|()| stdout.flush());
        if ok.is_err() {
            return;
        }
    }
}

fn handle(profile: WorkerProfile, req: &Request, payload: &[u8]) -> (Response, Vec<u8>) {
    if req.hook != 0 {
        #[cfg(feature = "sandbox-test-hooks")]
        return crate::hooks::run(req, payload);
        #[cfg(not(feature = "sandbox-test-hooks"))]
        std::process::exit(EXIT_PROTOCOL);
    }
    let caps = profile.caps();
    // İsteğin sınırları profilin tavanını aşamaz (ebeveyn de kırpar; işçi kendini korur).
    let limits = caps.clamp(&Limits {
        max_file_bytes: u64::from(caps.max_input_bytes),
        max_width: req.max_width,
        max_height: req.max_height,
        max_pixels: req.max_pixels,
        max_decode_bytes: req.max_decode_bytes,
        ..caps.limits
    });
    let opts = DecodeOptions { limits, target: req.target, animate: false };
    match decode_isolated(payload, &opts) {
        Ok(img) => {
            let (w, h) = img.size();
            if u64::from(w) * u64::from(h) > req.max_out_pixels.min(caps.max_out_pixels) {
                return (Response::error(ErrorCode::TooLarge, w, h), Vec::new());
            }
            // Yalnızca kap yapısı yürünür (piksel çözülmez); ilk kare verildi. Animasyonda kare
            // sayısı ve süre de burada, kilitli işçide sayılır (tel v4): ana süreç kabı yürümez.
            let resp = Response::ok(w, h);
            let resp = if matches!(is_animated(payload, &limits), Ok(true)) { resp.with_animation(animation_info(payload)) } else { resp };
            (resp, img.into_pixels())
        }
        Err(e) => {
            let (code, w, h) = ErrorCode::from_error(&e);
            (Response::error(code, w, h), Vec::new())
        }
    }
}

#[derive(Debug)]
struct LockdownError;

fn fail<E>(_: E) -> LockdownError {
    LockdownError
}

/// stdin/stdout/stderr dışındaki açık fd sayısı (miras; `read_dir`'in kendi fd'si düşülür).
/// Yalnızca sayı; yol okunmaz. Landlock'tan önce çağrılır (`/proc` sonra erişilemez).
fn inherited_fd_count() -> u16 {
    let Ok(dir) = std::fs::read_dir("/proc/self/fd") else { return 0 };
    let n = dir.flatten().filter(|e| e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()).is_some_and(|fd| fd > 2)).count();
    // Sayımda dizinin kendi fd'si de görünür.
    u16::try_from(n.saturating_sub(1)).unwrap_or(u16::MAX)
}

/// Profilin önceliği (en iyi çaba; başarısızlık kilidi bozmaz): düşük `nice` ve yüksek
/// `oom_score_adj`. Landlock'tan önce (`/proc` sonra erişilemez). Önceliği düşürmek ve
/// `oom_score_adj`'ı artırmak yetki istemez.
fn apply_priority(profile: WorkerProfile) {
    let caps = profile.caps();
    if caps.nice != 0 {
        let _ = rustix::process::setpriority_process(None, caps.nice);
    }
    if let Some(adj) = caps.oom_score_adj {
        let _ = std::fs::write("/proc/self/oom_score_adj", adj.to_string());
    }
}

fn lockdown(flags: Flags) -> Result<protocol::Handshake, LockdownError> {
    use rustix::process::{Resource, Rlimit, setrlimit};
    let caps = flags.profile.caps();
    let inherited_fds = inherited_fd_count();
    apply_priority(flags.profile);
    let lim = |r: Resource, soft: u64, hard: u64| setrlimit(r, Rlimit { current: Some(soft), maximum: Some(hard) }).map_err(fail);
    lim(Resource::As, caps.address_space, caps.address_space)?;
    lim(Resource::Cpu, caps.cpu_soft_secs, caps.cpu_hard_secs)?;
    lim(Resource::Fsize, 0, 0)?;
    lim(Resource::Nproc, 0, 0)?;
    lim(Resource::Core, 0, 0)?;
    lim(Resource::Memlock, 0, 0)?;
    rustix::thread::set_no_new_privs(true).map_err(fail)?;
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable).map_err(fail)?;
    let landlock = landlock_deny_all()?;
    lim(Resource::Nofile, 3, 3)?;
    // Std'nin stdin/stdout tamponları kilitten önce ayrılsın (ilk kullanımda tembel kurulum).
    let _ = std::io::stdin().lock();
    let _ = std::io::stdout().lock();
    let mut hs_flags = 0;
    if !flags.no_seccomp {
        seccomp()?;
        hs_flags |= protocol::FLAG_SECCOMP;
    }
    if cfg!(feature = "sandbox-test-hooks") {
        hs_flags |= protocol::FLAG_TEST_HOOKS;
    }
    Ok(protocol::Handshake { flags: hs_flags, landlock, inherited_fds, profile: flags.profile.code() })
}

/// Landlock kural kümesi; dönen değer el sıkışmadaki düzeydir. Hiç uygulanamazsa (Landlock'suz
/// çekirdek) hata: işçi [`EXIT_LOCKDOWN`] ile çıkar (bulgu 1c). Kısmi uygulama (eski ABI'de
/// bazı haklar yok) kabul edilir.
fn landlock_deny_all() -> Result<u8, LockdownError> {
    use landlock::{ABI, Access, AccessFs, AccessNet, Ruleset, RulesetAttr, RulesetStatus, Scope};
    let abi = ABI::V6;
    // Varsayılan uyumluluk "en iyi çaba": eski çekirdekte desteklenmeyen hak sessizce düşer.
    let status = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .map_err(fail)?
        .handle_access(AccessNet::from_all(abi))
        .map_err(fail)?
        .scope(Scope::from_all(abi))
        .map_err(fail)?
        .create()
        .map_err(fail)?
        .restrict_self()
        .map_err(fail)?;
    match status.ruleset {
        RulesetStatus::FullyEnforced => Ok(protocol::LANDLOCK_FULL),
        RulesetStatus::PartiallyEnforced => Ok(protocol::LANDLOCK_PARTIAL),
        RulesetStatus::NotEnforced => Err(LockdownError),
    }
}

/// İzin listesi: (sistem çağrısı, koşul kümeleri). Boş küme = koşulsuz izin; birden fazla
/// koşul = VE.
pub(crate) fn allowed_syscalls() -> Vec<(i64, Vec<seccompiler::SeccompCondition>)> {
    use seccompiler::SeccompCmpArgLen::{Dword, Qword};
    use seccompiler::SeccompCmpOp::{Eq, MaskedEq};
    use seccompiler::SeccompCondition as C;
    let c = |i, len, op, v| C::new(i, len, op, v).expect("geçerli seccomp koşulu");
    let no_exec = || c(2, Qword, MaskedEq(libc::PROT_EXEC as u64), 0);
    vec![
        (libc::SYS_read, vec![c(0, Dword, Eq, 0)]),
        (libc::SYS_write, vec![c(0, Dword, Eq, 1)]),
        // Anonim (fd = -1) ve çalıştırılamaz eşleme: miras fd eşlenemez, kod enjekte edilemez.
        (libc::SYS_mmap, vec![no_exec(), c(4, Dword, Eq, u64::from(u32::MAX))]),
        (libc::SYS_mprotect, vec![no_exec()]),
        (libc::SYS_munmap, vec![]),
        (libc::SYS_mremap, vec![]),
        (libc::SYS_brk, vec![]),
        (libc::SYS_madvise, vec![]),
        (libc::SYS_futex, vec![]),
        (libc::SYS_sched_yield, vec![]),
        (libc::SYS_sigaltstack, vec![]),
        (libc::SYS_rt_sigreturn, vec![]),
        (libc::SYS_rt_sigprocmask, vec![]),
        (libc::SYS_restart_syscall, vec![]),
        (libc::SYS_clock_gettime, vec![]),
        (libc::SYS_getrandom, vec![]),
        (libc::SYS_exit, vec![]),
        (libc::SYS_exit_group, vec![]),
    ]
}

fn seccomp() -> Result<(), LockdownError> {
    use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, SeccompRule, TargetArch};
    let arch: TargetArch = std::env::consts::ARCH.try_into().map_err(fail)?;
    let mut rules = std::collections::BTreeMap::new();
    for (nr, conds) in allowed_syscalls() {
        let r = if conds.is_empty() { vec![] } else { vec![SeccompRule::new(conds).map_err(fail)?] };
        rules.insert(nr, r);
    }
    let filter = SeccompFilter::new(rules, SeccompAction::KillProcess, SeccompAction::Allow, arch).map_err(fail)?;
    let prog: BpfProgram = filter.try_into().map_err(fail)?;
    seccompiler::apply_filter(&prog).map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_compiles_for_this_arch() {
        use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, SeccompRule, TargetArch};
        let arch: TargetArch = std::env::consts::ARCH.try_into().unwrap();
        let mut rules = std::collections::BTreeMap::new();
        for (nr, conds) in allowed_syscalls() {
            let r = if conds.is_empty() { vec![] } else { vec![SeccompRule::new(conds).unwrap()] };
            rules.insert(nr, r);
        }
        let f = SeccompFilter::new(rules, SeccompAction::KillProcess, SeccompAction::Allow, arch).unwrap();
        let prog: BpfProgram = f.try_into().unwrap();
        assert!(!prog.is_empty());
        // Tehlikeli çağrılar listede yok.
        let allowed: Vec<i64> = allowed_syscalls().into_iter().map(|(n, _)| n).collect();
        for nr in [libc::SYS_openat, libc::SYS_socket, libc::SYS_clone, libc::SYS_execve, libc::SYS_ioctl, libc::SYS_close, libc::SYS_ptrace] {
            assert!(!allowed.contains(&nr), "{nr}");
        }
    }

    #[test]
    fn serve_answers_and_stops_on_eof() {
        // Bu test ikilisi işçi kipine girer (yalnızca bu crate'in birim testleri).
        nuxut_image::enter_worker_mode();
        let svg: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"><rect width="2" height="2" fill="red"/></svg>"#;
        let req = Request {
            payload_len: svg.len() as u32,
            kind: nuxut_image::SniffedFormat::Svg,
            target: None,
            hook: 0,
            max_width: 64,
            max_height: 64,
            max_pixels: 4096,
            max_out_pixels: 4096,
            max_decode_bytes: 1 << 20,
        };
        let mut input = req.encode().to_vec();
        input.extend_from_slice(svg);
        let mut out = Vec::new();
        serve(WorkerProfile::Icon, &mut input.as_slice(), &mut out);
        let head: [u8; protocol::RESPONSE_HEADER_LEN] = out[..protocol::RESPONSE_HEADER_LEN].try_into().unwrap();
        let resp = Response::decode(&head);
        assert_eq!(resp.validate(&req), Ok(Some(16)));
        assert_eq!(out.len(), protocol::RESPONSE_HEADER_LEN + 16);
    }

    /// `<image href>` düşmanca külliyatı (testler arası ortak): `(ad, href)`.
    pub(crate) fn hostile_hrefs(existing_png: &std::path::Path) -> Vec<(&'static str, String)> {
        let nested_svg = "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI4IiBoZWlnaHQ9IjgiPjxyZWN0IHdpZHRoPSI4IiBoZWlnaHQ9IjgiLz48L3N2Zz4=";
        vec![
            ("file-passwd", "file:///etc/passwd".into()),
            ("file-existing", format!("file://{}", existing_png.display())),
            ("abs-existing", existing_png.display().to_string()),
            ("rel-parent", "../x.png".into()),
            ("rel-plain", "x.png".into()),
            ("dev-zero", "/dev/zero".into()),
            ("http", "http://127.0.0.1:9/a.png".into()),
            ("https", "https://example.com/a.png".into()),
            ("ftp", "ftp://example.com/a.png".into()),
            ("net-path", "//host/a.png".into()),
            ("data-svg", format!("data:image/svg+xml;base64,{nested_svg}")),
        ]
    }

    pub(crate) fn svg_with(href: Option<&str>) -> Vec<u8> {
        let image = href.map(|h| format!(r#"<image x="0" y="0" width="16" height="16" xlink:href="{h}"/>"#)).unwrap_or_default();
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="16" height="16"><rect width="16" height="16" fill="red"/>{image}<circle cx="8" cy="8" r="4" fill="blue"/></svg>"#
        )
        .into_bytes()
    }

    /// Talep, test 1 (süreç içi, işçi kipi): dış kaynaklı `<image>` hiçbir şey
    /// eklemez — çıktı `<image>`'sız SVG'ninkiyle bayt bayt aynı. data: PNG ise çizilir (fark).
    #[test]
    fn image_href_never_loads_anything_in_worker_mode() {
        nuxut_image::enter_worker_mode();
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("x.png");
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 255, 0, 255])).write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        std::fs::write(&png, &bytes).unwrap();
        let opts = DecodeOptions { limits: Limits::ICON, target: None, animate: false };
        let base = decode_isolated(&svg_with(None), &opts).unwrap();
        for (name, href) in hostile_hrefs(&png) {
            let got = decode_isolated(&svg_with(Some(&href)), &opts).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(got == base, "{name}: <image> bir şey yükledi");
        }
        // Pozitif kontrol: aynı PNG data: ile gömülünce çizilir (sınırlar içinde).
        let data = format!("data:image/png;base64,{}", base64(&bytes));
        assert!(decode_isolated(&svg_with(Some(&data)), &opts).unwrap() != base, "data: PNG çizilmeli");
    }

    fn base64(b: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in b.chunks(3) {
            let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
            for i in 0..4 {
                s.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
            }
        }
        s
    }
}
