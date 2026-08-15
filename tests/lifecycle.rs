use remind_me::app::{BackgroundHold, HoldChange};

#[test]
fn background_hold_changes_only_when_pending_state_crosses_the_boundary() {
    let mut hold = BackgroundHold::default();

    assert_eq!(hold.update(false), None);
    assert_eq!(hold.update(true), Some(HoldChange::Hold));
    assert_eq!(hold.update(true), None);
    assert_eq!(hold.update(false), Some(HoldChange::Release));
    assert_eq!(hold.update(false), None);
}
