//! `nuxut-video` bütünleşik testleri: tek test ikilisi (`cargo test -p nuxut-video --test it`).
//! Zaman aşımında süreç temizliği ayrı ikilide (`import_timeout.rs`): iş parçacığı sayımı
//! süreç genelindedir, başka testlerden etkilenmesin.

mod import;
mod import_budget;
mod isolation;
mod untrusted_media;
