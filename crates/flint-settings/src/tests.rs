use proptest::prelude::*;
use serde_json::json;

use super::*;

fn with(entries: &[(&str, &str, u64)]) -> Settings {
    let mut settings = Settings::default();
    for (key, folder, at) in entries {
        settings.set_folder(key, folder, *at).unwrap();
    }
    settings
}

#[test]
fn folders_are_tidied_and_the_unusable_refused() {
    assert_eq!(normalize_folder("Images").as_deref(), Some("Images"));
    assert_eq!(normalize_folder("/Images/").as_deref(), Some("Images"));
    assert_eq!(
        normalize_folder(" Attachments // pictures ").as_deref(),
        Some("Attachments/pictures")
    );
    assert_eq!(normalize_folder("").as_deref(), Some(""));
    assert_eq!(normalize_folder("  /  ").as_deref(), Some(""));
    for bad in [
        "..",
        "a/../b",
        ".",
        "a/./b",
        ".hidden",
        "a/.git",
        "back\\slash",
        "what?",
        "a:b",
        "CON",
        "x/nul.txt",
        "trailing.",
    ] {
        assert_eq!(normalize_folder(bad), None, "{bad}");
    }
}

#[test]
fn unset_settings_have_their_defaults() {
    let typed = Settings::default().typed();
    assert_eq!(typed, StoreSettings::default());
    assert_eq!(typed.attachment_folder, "Images");
    assert_eq!(typed.new_note_folder, "");
}

#[test]
fn a_setting_can_be_set_to_the_top_level_and_that_is_not_the_default() {
    let settings = with(&[(ATTACHMENT_FOLDER, "", 10)]);
    assert_eq!(
        settings.typed().attachment_folder,
        "",
        "set to nothing on purpose"
    );
}

#[test]
fn setting_an_unusable_folder_is_refused_and_changes_nothing() {
    let mut settings = with(&[(ATTACHMENT_FOLDER, "Pics", 1)]);
    assert!(matches!(
        settings.set_folder(ATTACHMENT_FOLDER, "../x", 2),
        Err(Error::InvalidFolder(_))
    ));
    assert_eq!(settings.typed().attachment_folder, "Pics");
}

#[test]
fn a_stored_value_that_is_not_a_usable_folder_counts_as_not_set() {
    let text = r#"{"version":1,"settings":{"attachment_folder":{"value":"../escape","at":5},"new_note_folder":{"value":7,"at":5}}}"#;
    let Parsed::Current(settings) = parse(text).unwrap() else {
        panic!()
    };
    assert_eq!(settings.typed(), StoreSettings::default());
}

#[test]
fn the_later_change_wins_and_each_setting_is_decided_on_its_own() {
    let mac = with(&[
        (ATTACHMENT_FOLDER, "MacPics", 100),
        (NEW_NOTE_FOLDER, "MacInbox", 300),
    ]);
    let phone = with(&[
        (ATTACHMENT_FOLDER, "PhonePics", 200),
        (NEW_NOTE_FOLDER, "PhoneInbox", 250),
    ]);
    let merged = mac.merge(&phone).typed();
    assert_eq!(merged.attachment_folder, "PhonePics");
    assert_eq!(merged.new_note_folder, "MacInbox");
    assert_eq!(mac.merge(&phone), phone.merge(&mac), "whichever side asks");
}

#[test]
fn settings_one_side_has_never_heard_of_are_kept() {
    // an older Flint merging with a newer one's file must not drop what it does not understand
    let newer_file = r#"{"version":1,"settings":{"future_setting":{"value":{"a":[1,2]},"at":50}}}"#;
    let Parsed::Current(theirs) = parse(newer_file).unwrap() else {
        panic!()
    };
    let merged = with(&[(ATTACHMENT_FOLDER, "Pics", 10)]).merge(&theirs);
    assert!(merged.to_json().contains("future_setting"));
    assert!(merged.to_json().contains("\"a\""));
    assert_eq!(merged.typed().attachment_folder, "Pics");
}

#[test]
fn a_file_round_trips() {
    let settings = with(&[
        (ATTACHMENT_FOLDER, "Pics/pasted", 123),
        (NEW_NOTE_FOLDER, "Inbox", 456),
    ]);
    assert_eq!(
        parse(&settings.to_json()).unwrap(),
        Parsed::Current(settings)
    );
}

#[test]
fn a_file_from_a_newer_version_is_reported_not_read() {
    // even if its shape is one this version could not read at all
    let newer = json!({"version": VERSION + 1, "settings": [1, 2, 3], "other": true}).to_string();
    assert_eq!(parse(&newer).unwrap(), Parsed::Newer(VERSION + 1));
}

#[test]
fn damaged_files_are_errors() {
    for bad in [
        "",
        "not json",
        "[]",
        "42",
        r#"{"settings":{}}"#,
        r#"{"version":"one","settings":{}}"#,
        r#"{"version":1}"#,
        r#"{"version":1,"settings":7}"#,
    ] {
        assert!(matches!(parse(bad), Err(Error::Damaged(_))), "{bad}");
    }
}

fn arbitrary_settings() -> impl Strategy<Value = Settings> {
    let key = prop::sample::select(vec![ATTACHMENT_FOLDER, NEW_NOTE_FOLDER, "future"]);
    let folder = prop::sample::select(vec!["", "Images", "Pics", "a/b", "Inbox"]);
    prop::collection::vec((key, folder, 0u64..6), 0..8).prop_map(|entries| {
        let mut settings = Settings::default();
        for (key, folder, at) in entries {
            // all the same key at the same time with different folders is the case the tie-break exists for
            settings.0.insert(
                key.to_owned(),
                Entry {
                    value: json!(folder),
                    at,
                },
            );
        }
        settings
    })
}

proptest! {
    // What makes "no conflicts" true: devices can sync in any order, any number of times, and agree.
    #[test]
    fn merging_is_commutative(a in arbitrary_settings(), b in arbitrary_settings()) {
        prop_assert_eq!(a.merge(&b), b.merge(&a));
    }

    #[test]
    fn merging_is_associative(a in arbitrary_settings(), b in arbitrary_settings(), c in arbitrary_settings()) {
        prop_assert_eq!(a.merge(&b).merge(&c), a.merge(&b.merge(&c)));
    }

    #[test]
    fn merging_is_idempotent(a in arbitrary_settings(), b in arbitrary_settings()) {
        let once = a.merge(&b);
        prop_assert_eq!(once.merge(&b), once.clone());
        prop_assert_eq!(once.merge(&a), once);
    }

    #[test]
    fn a_merge_never_loses_a_setting_or_a_newer_value(a in arbitrary_settings(), b in arbitrary_settings()) {
        let merged = a.merge(&b);
        for side in [&a, &b] {
            for (key, entry) in &side.0 {
                let kept = merged.0.get(key).expect("every setting survives");
                prop_assert!(kept.at >= entry.at);
            }
        }
    }
}
