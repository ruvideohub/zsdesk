//! Fault injection for a background thread that cannot be created.
use super::*;

fn refuse(_: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    Err(std::io::Error::other("injected spawn failure"))
}

#[test]
fn failed_fetch_unblocks_slot_and_reports_failure() {
    let mut hub = Hub::new(HWND::default());
    hub.slots.insert("test".into(), Slot { busy: true, ..Default::default() });
    hub.run_with(Job::Fetch("test".into(), Params::new()), refuse);
    assert!(hub.take_replies());
    let slot = &hub.slots["test"];
    assert!(!slot.busy);
    assert!(matches!(&slot.err, Some(FeedError::Failed(_))));
    assert!(slot.at.is_some());
}

#[test]
fn failed_radio_jobs_leave_loading_and_show_error() {
    let mut hub = Hub::new(HWND::default());
    hub.radio.loading = true;
    hub.radio.pending_play = true;
    hub.run_with(Job::RadioCatalogue, refuse);
    assert!(hub.take_replies());
    assert!(!hub.radio.loading);
    assert!(!hub.radio.pending_play);
    assert!(hub.radio.error);
    hub.radio.loading = true;
    hub.run_with(Job::RadioChoose, refuse);
    assert!(hub.take_replies());
    assert!(!hub.radio.loading);
    assert!(hub.radio.import_error);
}

#[test]
fn failed_brightness_worker_releases_busy_and_marks_unavailable() {
    let mut hub = Hub::new(HWND::default());
    hub.brightness_busy.store(true, Ordering::SeqCst);
    hub.run_with(Job::SetBrightness, refuse);
    assert!(!hub.brightness_busy.load(Ordering::SeqCst));
    assert!(hub.take_replies());
    assert!(hub.brightness_unsupported);
}

#[test]
fn failed_location_lookup_resolves_instead_of_stalling() {
    let mut hub = Hub::new(HWND::default());
    hub.run_with(Job::Geocode("Test City".into()), refuse);
    assert!(hub.take_replies());
    assert_eq!(hub.geocoded, Some(("Test City".into(), None)));
}
