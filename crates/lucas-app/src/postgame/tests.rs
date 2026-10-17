use super::*;
use lucas_store::{DATABASE_NAME, Settings};

fn stored_card(directory: &Path) -> PostGameCard {
    let mut store = Store::open_in(directory).unwrap();
    let summary = demo_summary();
    store.save_summary(&summary).unwrap();
    store.save_settings(&Settings::default()).unwrap();
    PostGameCard::from_store(store, &summary.id).unwrap()
}

#[test]
fn runtime_deletion_is_deferred_until_the_parent_releases_its_state() {
    let directory = tempfile::tempdir().unwrap();
    let mut card = stored_card(directory.path());
    let request = Arc::new(AtomicBool::new(false));
    card.delete_request = Some(Arc::clone(&request));

    card.delete_data();

    assert!(request.load(Ordering::Acquire));
    assert!(card.store.is_none());
    let store = Store::open_in(directory.path()).unwrap();
    assert!(store.latest_summary().unwrap().is_some());
    assert!(directory.path().join("settings.toml").exists());
    drop(card);
    store.delete_all_data().unwrap();
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn deletion_retries_without_a_handle_database_or_summary() {
    let directory = tempfile::tempdir().unwrap();
    let mut card = stored_card(directory.path());
    drop(card.store.take());
    std::fs::remove_file(directory.path().join(DATABASE_NAME)).unwrap();
    card.summary = None;
    card.notice = Some("Deletion was incomplete".into());

    card.delete_data();

    assert!(card.notice.is_none());
    assert!(card.summary.is_none());
    assert!(card.probes.is_empty());
    assert!(!directory.path().join(DATABASE_NAME).exists());
    assert!(!directory.path().join("settings.toml").exists());
    card.delete_data();
    assert!(card.notice.is_none());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn failed_deletion_can_be_retried_on_the_same_card() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("data");
    let moved = root.path().join("moved");
    let mut card = stored_card(&directory);
    std::fs::rename(&directory, &moved).unwrap();

    card.delete_data();
    assert!(card.notice.is_some());
    assert!(card.store.is_none());
    assert!(card.data_folder.is_some());

    std::fs::rename(&moved, &directory).unwrap();
    std::fs::remove_file(directory.join(DATABASE_NAME)).unwrap();
    card.delete_data();
    assert!(card.notice.is_none());
    assert!(card.summary.is_none());
    assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
}

#[cfg(windows)]
#[test]
fn locked_settings_can_be_deleted_after_database_removal() {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = tempfile::tempdir().unwrap();
    let mut card = stored_card(directory.path());
    let locked = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(directory.path().join("settings.toml"))
        .unwrap();

    card.delete_data();
    assert!(card.notice.is_some());
    assert!(card.store.is_none());
    assert!(card.data_folder.is_some());
    assert!(!directory.path().join(DATABASE_NAME).exists());
    assert!(directory.path().join("settings.toml").exists());

    drop(locked);
    card.delete_data();
    assert!(card.notice.is_none());
    assert!(card.summary.is_none());
    assert!(card.probes.is_empty());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
