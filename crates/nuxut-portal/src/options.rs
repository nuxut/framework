//! Seçenekler → FileChooser `a{sv}` sözlüğü (xdg-desktop-portal belgesi,
//! `org.freedesktop.portal.FileChooser`).

use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use zbus::zvariant::Value;

use crate::{Filter, FilterRule, OpenFileOptions, SaveFileOptions};

/// `a{sv}`; anahtarlar sabit.
pub(crate) type VarDict = HashMap<&'static str, Value<'static>>;

/// Filtre `(sa(us))`: ad + (tür, desen); tür 0 = glob, 1 = MIME.
type FilterTuple = (String, Vec<(u32, String)>);

fn filter_tuple(filter: &Filter) -> FilterTuple {
    let rules = filter
        .rules
        .iter()
        .map(|rule| match rule {
            FilterRule::Glob(p) => (0, p.clone()),
            FilterRule::Mime(m) => (1, m.clone()),
        })
        .collect();
    (filter.name.clone(), rules)
}

/// Yol baytları + NUL (`ay`; portal belgesi: "null-terminated").
fn path_bytes(path: &Path) -> Vec<u8> {
    let mut bytes = path.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    bytes
}

fn common(
    dict: &mut VarDict,
    token: &str,
    accept_label: Option<&String>,
    modal: bool,
    filters: &[Filter],
    current_filter: Option<usize>,
    current_folder: Option<&Path>,
) {
    dict.insert("handle_token", Value::from(token.to_owned()));
    dict.insert("modal", Value::from(modal));
    if let Some(label) = accept_label {
        dict.insert("accept_label", Value::from(label.clone()));
    }
    if !filters.is_empty() {
        let list: Vec<FilterTuple> = filters.iter().map(filter_tuple).collect();
        dict.insert("filters", Value::from(list));
    }
    if let Some(current) = current_filter.and_then(|i| filters.get(i)) {
        dict.insert(
            "current_filter",
            Value::from(zbus::zvariant::Structure::from(filter_tuple(current))),
        );
    }
    if let Some(folder) = current_folder {
        dict.insert("current_folder", Value::from(path_bytes(folder)));
    }
}

pub(crate) fn open_file(options: &OpenFileOptions, token: &str) -> VarDict {
    let mut dict = VarDict::new();
    common(
        &mut dict,
        token,
        options.accept_label.as_ref(),
        options.modal,
        &options.filters,
        options.current_filter,
        options.current_folder.as_deref(),
    );
    dict.insert("multiple", Value::from(options.multiple));
    dict.insert("directory", Value::from(options.directory));
    dict
}

pub(crate) fn save_file(options: &SaveFileOptions, token: &str) -> VarDict {
    let mut dict = VarDict::new();
    common(
        &mut dict,
        token,
        options.accept_label.as_ref(),
        options.modal,
        &options.filters,
        options.current_filter,
        options.current_folder.as_deref(),
    );
    if let Some(name) = &options.current_name {
        dict.insert("current_name", Value::from(name.clone()));
    }
    dict
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_dict_minimal() {
        let dict = open_file(&OpenFileOptions::new("Aç"), "nuxut_1");
        let mut keys: Vec<_> = dict.keys().copied().collect();
        keys.sort_unstable();
        assert_eq!(keys, ["directory", "handle_token", "modal", "multiple"]);
        assert_eq!(dict["handle_token"], Value::from("nuxut_1"));
        assert_eq!(dict["modal"], Value::from(true));
        assert_eq!(dict["multiple"], Value::from(false));
        assert_eq!(dict["directory"], Value::from(false));
    }

    #[test]
    fn open_dict_full() {
        let options = OpenFileOptions {
            title: "VPN içe aktar".into(),
            accept_label: Some("İçe aktar".into()),
            multiple: true,
            directory: false,
            filters: vec![
                Filter::new("OpenVPN").glob("*.ovpn").glob("*.conf"),
                Filter::new("Resimler").mime("image/*"),
            ],
            current_filter: Some(1),
            current_folder: Some("/home/u/İndirilenler".into()),
            modal: false,
        };
        let dict = open_file(&options, "t");
        assert_eq!(dict["accept_label"], Value::from("İçe aktar"));
        assert_eq!(dict["multiple"], Value::from(true));
        assert_eq!(dict["modal"], Value::from(false));
        assert_eq!(dict["filters"].value_signature(), "a(sa(us))");
        assert_eq!(dict["current_filter"].value_signature(), "(sa(us))");
        let filters: Vec<FilterTuple> = dict["filters"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(
            filters,
            vec![
                (
                    "OpenVPN".to_owned(),
                    vec![(0, "*.ovpn".to_owned()), (0, "*.conf".to_owned())]
                ),
                ("Resimler".to_owned(), vec![(1, "image/*".to_owned())]),
            ]
        );
        let current: FilterTuple = dict["current_filter"]
            .try_clone()
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(current.0, "Resimler");
        assert_eq!(dict["current_folder"].value_signature(), "ay");
        let folder: Vec<u8> = dict["current_folder"]
            .try_clone()
            .unwrap()
            .try_into()
            .unwrap();
        assert_eq!(folder, b"/home/u/\xc4\xb0ndirilenler\0");
    }

    #[test]
    fn invalid_current_filter_is_ignored() {
        let mut options = OpenFileOptions::new("x");
        options.current_filter = Some(3);
        assert!(!open_file(&options, "t").contains_key("current_filter"));
        options.filters = vec![Filter::new("a").glob("*")];
        assert!(open_file(&options, "t").contains_key("filters"));
        assert!(!open_file(&options, "t").contains_key("current_filter"));
    }

    #[test]
    fn save_dict() {
        let mut options = SaveFileOptions::new("Kaydet");
        options.current_name = Some("yedek.toml".into());
        options.current_folder = Some("/tmp".into());
        let dict = save_file(&options, "t");
        let mut keys: Vec<_> = dict.keys().copied().collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["current_folder", "current_name", "handle_token", "modal"]
        );
        assert_eq!(dict["current_name"], Value::from("yedek.toml"));
        assert!(!dict.contains_key("multiple"));
    }
}
