use std::rc::Rc;
use std::{cell::Cell, ops::Deref};

use adw::prelude::*;
use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Utc};
use cue::{
    model::{CanvasEntry, DeletedCanvasItem, Reminder},
    repository::{ReminderRepository, RepositoryError, SqliteReminderRepository},
    resources,
    scheduler::{Clock, NotificationError, ReminderNotifier},
    service::ReminderService,
    ui,
};
use uuid::Uuid;

struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

struct NoopNotifier;

impl ReminderNotifier for NoopNotifier {
    fn send(&self, _id: &str, _reminder: &Reminder) -> Result<(), NotificationError> {
        Ok(())
    }

    fn withdraw(&self, _id: &str) {}
}

struct FailingCanvasSaveRepository {
    inner: SqliteReminderRepository,
    fail_saves: Cell<bool>,
}

impl FailingCanvasSaveRepository {
    fn in_memory() -> Self {
        Self {
            inner: SqliteReminderRepository::in_memory().unwrap(),
            fail_saves: Cell::new(false),
        }
    }

    fn set_fail_saves(&self, fail: bool) {
        self.fail_saves.set(fail);
    }
}

impl Deref for FailingCanvasSaveRepository {
    type Target = SqliteReminderRepository;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

macro_rules! delegate_repository_method {
    ($name:ident($($argument:ident: $type:ty),* $(,)?) -> $output:ty) => {
        fn $name(&self, $($argument: $type),*) -> Result<$output, RepositoryError> {
            self.inner.$name($($argument),*)
        }
    };
}

impl ReminderRepository for FailingCanvasSaveRepository {
    delegate_repository_method!(insert(reminder: &Reminder) -> ());
    delegate_repository_method!(restore(reminder: &Reminder) -> ());
    delegate_repository_method!(get(id: Uuid) -> Reminder);
    delegate_repository_method!(list_active() -> Vec<Reminder>);
    delegate_repository_method!(list_history() -> Vec<Reminder>);
    delegate_repository_method!(edit(
        id: Uuid,
        message: &str,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Reminder);
    delegate_repository_method!(mark_notified(id: Uuid, now: DateTime<Utc>) -> Reminder);
    delegate_repository_method!(snooze(id: Uuid, now: DateTime<Utc>) -> Reminder);
    delegate_repository_method!(snooze_canvas_reminder(
        id: Uuid,
        now: DateTime<Utc>,
        working_text: Option<(Uuid, &str)>,
    ) -> Reminder);
    delegate_repository_method!(complete(id: Uuid, now: DateTime<Utc>) -> Reminder);
    delegate_repository_method!(delete(id: Uuid) -> Reminder);
    delegate_repository_method!(clear_history() -> usize);
    delegate_repository_method!(list_canvas_entries() -> Vec<CanvasEntry>);
    delegate_repository_method!(append_canvas_entry(
        message: &str,
        reminder: Option<&Reminder>,
        now: DateTime<Utc>,
    ) -> CanvasEntry);

    fn save_canvas_working_text(
        &self,
        id: Uuid,
        text: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<(), RepositoryError> {
        if self.fail_saves.get() {
            return Err(RepositoryError::Database(
                rusqlite::Error::ExecuteReturnedResults,
            ));
        }
        self.inner.save_canvas_working_text(id, text, now)
    }

    delegate_repository_method!(load_canvas_draft() -> String);

    fn save_canvas_draft(&self, text: &str) -> Result<(), RepositoryError> {
        if self.fail_saves.get() {
            return Err(RepositoryError::Database(
                rusqlite::Error::ExecuteReturnedResults,
            ));
        }
        self.inner.save_canvas_draft(text)
    }

    delegate_repository_method!(get_canvas_entry(id: Uuid) -> CanvasEntry);
    delegate_repository_method!(attach_canvas_reminder(
        entry_id: Uuid,
        reminder: &Reminder,
        now: DateTime<Utc>,
    ) -> CanvasEntry);
    delegate_repository_method!(detach_canvas_reminder(
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> (CanvasEntry, Reminder));
    delegate_repository_method!(complete_canvas_reminder(
        reminder_id: Uuid,
        now: DateTime<Utc>,
    ) -> Reminder);
    delegate_repository_method!(delete_canvas_entry(id: Uuid) -> DeletedCanvasItem);
    delegate_repository_method!(restore_canvas_item(item: &DeletedCanvasItem) -> ());
    delegate_repository_method!(update_canvas_note(
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> CanvasEntry);
    delegate_repository_method!(rename_canvas_reminder(
        entry_id: Uuid,
        message: &str,
        now: DateTime<Utc>,
    ) -> (CanvasEntry, Reminder));
    delegate_repository_method!(reschedule_canvas_reminder(
        entry_id: Uuid,
        message: &str,
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> (CanvasEntry, Reminder));
}

#[test]
fn gtk_smoke_covers_canvas_entries_and_secondary_navigation() {
    gtk::init().unwrap();
    adw::init().unwrap();
    resources::register().unwrap();
    let application = adw::Application::builder()
        .application_id("io.github.wuzi.Cue.Test")
        .build();
    application.register(None::<&gio::Cancellable>).unwrap();

    let widgets = ui::build_window(&application).unwrap();
    assert_eq!(widgets.window.title().as_deref(), Some("Cue"));
    assert_eq!(widgets.window.default_width(), 560);
    assert_eq!(widgets.window.default_height(), 540);
    assert_eq!(widgets.window.width_request(), 360);
    assert_eq!(widgets.window.height_request(), 500);
    assert_eq!(
        widgets.navigation_view.visible_page_tag().as_deref(),
        Some("canvas")
    );
    assert_eq!(
        widgets.active_list_button.icon_name().as_deref(),
        Some("view-list-symbolic")
    );
    assert!(find_label_containing(widgets.window.upcast_ref(), "Write a note").is_none());
    assert!(find_icon_button(widgets.window.upcast_ref(), "list-add-symbolic").is_none());
    assert!(find_css_class(widgets.window.upcast_ref(), "schedule-preview").is_none());

    let now = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
    let repository = Rc::new(FailingCanvasSaveRepository::in_memory());
    let service = Rc::new(ReminderService::new(
        repository.clone(),
        Rc::new(FixedClock(now)),
        Rc::new(NoopNotifier),
    ));
    let window = ui::MainWindow::new(&application, service.clone(), || {}, || {}).unwrap();
    window.present();
    drain_main_context();

    let draft = find_css_class(window.widget().upcast_ref(), "canvas-draft")
        .unwrap()
        .downcast::<gtk::TextView>()
        .unwrap();
    assert_eq!(draft.wrap_mode(), gtk::WrapMode::WordChar);
    assert!(!draft.accepts_tab());
    draft.grab_focus();
    drain_main_context();
    assert!(window_focuses(window.widget(), draft.upcast_ref()));
    draft.buffer().set_text("Schedule @");
    drain_main_context();
    let suggestions = find_css_class(window.widget().upcast_ref(), "schedule-suggestions")
        .expect("schedule suggestions")
        .downcast::<gtk::Popover>()
        .unwrap();
    let (has_anchor, anchor) = suggestions.pointing_to();
    assert!(has_anchor, "suggestions must point to the text caret");
    assert!(
        anchor.y() < draft.height() / 2,
        "the first-line anchor must not use the expanding editor's bottom edge"
    );
    let active_list_button =
        find_icon_button(window.widget().upcast_ref(), "view-list-symbolic").unwrap();
    active_list_button.grab_focus();
    wait_until(|| find_css_class(window.widget().upcast_ref(), "schedule-suggestions").is_none());
    assert!(
        find_css_class(window.widget().upcast_ref(), "schedule-suggestions").is_none(),
        "suggestions must close when focus leaves their editor"
    );
    draft.grab_focus();
    draft.buffer().set_text("Schedule");
    draft.buffer().set_text("Schedule @");
    drain_main_context();
    draft.buffer().set_text("First line\nSchedule @");
    draft.buffer().place_cursor(&draft.buffer().end_iter());
    wait_until(|| {
        find_css_class(window.widget().upcast_ref(), "schedule-suggestions")
            .and_then(|widget| widget.downcast::<gtk::Popover>().ok())
            .is_some_and(|popover| popover.pointing_to().1.y() > anchor.y())
    });
    let suggestions = find_css_class(window.widget().upcast_ref(), "schedule-suggestions")
        .unwrap()
        .downcast::<gtk::Popover>()
        .unwrap();
    let multiline_anchor = suggestions.pointing_to().1;
    assert!(
        multiline_anchor.y() > anchor.y(),
        "the suggestion anchor must follow the caret onto later lines"
    );
    draft
        .buffer()
        .set_text("Schedule this reminder after changing the available width with several words @");
    draft.buffer().place_cursor(&draft.buffer().end_iter());
    drain_main_context();
    std::thread::sleep(std::time::Duration::from_millis(30));
    drain_main_context();
    let suggestions = find_css_class(window.widget().upcast_ref(), "schedule-suggestions")
        .unwrap()
        .downcast::<gtk::Popover>()
        .unwrap();
    let wide_anchor = suggestions.pointing_to().1;
    let draft_row = draft.parent().unwrap();
    draft_row.set_width_request(180);
    draft_row.set_halign(gtk::Align::Start);
    wait_until(|| draft.width() <= 180);
    assert!(draft.width() <= 180, "the test editor must reflow");
    wait_until(|| suggestions.pointing_to().1.y() > wide_anchor.y());
    assert!(
        suggestions.pointing_to().1.y() > wide_anchor.y(),
        "the suggestion anchor must follow allocation-driven word wrapping"
    );
    draft_row.set_width_request(-1);
    draft_row.set_halign(gtk::Align::Fill);
    wait_until(|| draft.width() > 300);
    assert!(draft.width() > 300, "the test editor must expand again");
    let suggestions = find_css_class(window.widget().upcast_ref(), "schedule-suggestions")
        .unwrap()
        .downcast::<gtk::Popover>()
        .unwrap();
    let suggestion_list = find_descendant::<gtk::ListBox>(suggestions.upcast_ref()).unwrap();
    let custom_row = suggestion_list.row_at_index(5).unwrap();
    suggestion_list.emit_by_name::<()>("row-activated", &[&custom_row]);
    drain_main_context();
    let picker = find_calendar_popover(window.widget().upcast_ref()).expect("draft picker");
    let wide_picker_anchor = picker.pointing_to().1;
    std::thread::sleep(std::time::Duration::from_millis(30));
    drain_main_context();
    draft_row.set_width_request(180);
    draft_row.set_halign(gtk::Align::Start);
    wait_until(|| draft.width() <= 180);
    wait_until(|| picker.pointing_to().1.y() > wide_picker_anchor.y());
    assert!(
        picker.pointing_to().1.y() > wide_picker_anchor.y(),
        "the custom picker anchor must follow allocation-driven word wrapping"
    );
    find_button_with_label(picker.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    draft_row.set_width_request(-1);
    draft_row.set_halign(gtk::Align::Fill);
    drain_main_context();
    draft.buffer().set_text("A plain note");
    gtk::prelude::WidgetExt::activate_action(window.widget(), "win.commit-canvas", None).unwrap();
    drain_main_context();

    let entries = repository.list_canvas_entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].message, "A plain note");
    assert_eq!(entries[0].reminder_id, None);
    assert_eq!(
        find_all_css(window.widget().upcast_ref(), "canvas-entry").len(),
        1
    );
    let draft = find_css_class(window.widget().upcast_ref(), "canvas-draft")
        .unwrap()
        .downcast::<gtk::TextView>()
        .unwrap();
    assert_eq!(buffer_text(&draft.buffer()), "");

    let saved_note = find_text_view_containing(window.widget().upcast_ref(), "A plain note")
        .expect("saved note editor");
    saved_note.buffer().set_text("@garbage");
    saved_note.grab_focus();
    gtk::prelude::WidgetExt::activate_action(window.widget(), "win.commit-canvas", None).unwrap();
    drain_main_context();
    assert_eq!(repository.list_canvas_entries().unwrap().len(), 1);
    assert_eq!(buffer_text(&saved_note.buffer()), "@garbage");
    saved_note.buffer().set_text("A plain note edited");
    saved_note
        .buffer()
        .place_cursor(&saved_note.buffer().iter_at_offset(6));
    let old_draft = draft.downgrade();
    let old_saved_note = saved_note.downgrade();
    drop(draft);
    drop(saved_note);
    for _ in 0..20 {
        window.refresh();
        drain_main_context();
        let focused =
            find_text_view_containing(window.widget().upcast_ref(), "A plain note edited")
                .expect("refreshed saved note editor");
        assert!(window_focuses(window.widget(), focused.upcast_ref()));
        assert_eq!(focused.buffer().cursor_position(), 6);
    }
    assert!(old_draft.upgrade().is_some());
    assert!(old_saved_note.upgrade().is_some());
    let saved_note = find_text_view_containing(window.widget().upcast_ref(), "A plain note edited")
        .expect("refreshed saved note editor");
    assert!(window_focuses(window.widget(), saved_note.upcast_ref()));
    assert_eq!(saved_note.buffer().cursor_position(), 6);
    assert_eq!(
        repository.list_canvas_entries().unwrap()[0]
            .working_text
            .as_deref(),
        Some("A plain note edited")
    );
    let active_list_button =
        find_icon_button(window.widget().upcast_ref(), "view-list-symbolic").unwrap();
    active_list_button.grab_focus();
    window.refresh();
    drain_main_context();
    assert!(window_focuses(
        window.widget(),
        active_list_button.upcast_ref()
    ));
    saved_note.grab_focus();
    saved_note.buffer().set_text("Survives a save failure");
    saved_note
        .buffer()
        .place_cursor(&saved_note.buffer().iter_at_offset(8));
    repository.set_fail_saves(true);
    window.refresh();
    drain_main_context();
    assert_eq!(buffer_text(&saved_note.buffer()), "Survives a save failure");
    assert!(window_focuses(window.widget(), saved_note.upcast_ref()));
    assert_eq!(saved_note.buffer().cursor_position(), 8);
    assert_eq!(
        repository.list_canvas_entries().unwrap()[0]
            .working_text
            .as_deref(),
        Some("A plain note edited")
    );
    repository.set_fail_saves(false);
    window.refresh();
    saved_note.buffer().set_text("A plain note edited");
    window.refresh();
    drain_main_context();
    let draft = find_css_class(window.widget().upcast_ref(), "canvas-draft")
        .unwrap()
        .downcast::<gtk::TextView>()
        .unwrap();
    draft.grab_focus();

    draft.buffer().set_text("Old task @2020-01-01 9am");
    drain_main_context();
    assert!(find_label_containing(window.widget().upcast_ref(), "future").is_some());
    gtk::prelude::WidgetExt::activate_action(window.widget(), "win.commit-canvas", None).unwrap();
    drain_main_context();
    assert_eq!(repository.list_canvas_entries().unwrap().len(), 1);
    assert_eq!(buffer_text(&draft.buffer()), "Old task @2020-01-01 9am");

    draft.buffer().set_text("Call Ada @in 30 minutes");
    drain_main_context();
    let draft_schedule = draft.buffer().iter_at_offset(10);
    assert!(
        draft_schedule
            .tags()
            .iter()
            .any(|tag| tag.name().as_deref() == Some("draft-schedule"))
    );
    drop(draft);
    drop(saved_note);
    gtk::prelude::WidgetExt::activate_action(window.widget(), "win.commit-canvas", None).unwrap();
    drain_main_context();
    assert!(old_draft.upgrade().is_none());
    assert!(old_saved_note.upgrade().is_none());

    let reminders = repository.list_active().unwrap();
    let edited_note = repository
        .list_canvas_entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.reminder_id.is_none())
        .unwrap();
    assert_eq!(
        edited_note.working_text.as_deref(),
        Some("A plain note edited")
    );
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0].message, "Call Ada");
    assert_eq!(reminders[0].due_at, now + Duration::minutes(30));
    let registered = find_text_view_containing(window.widget().upcast_ref(), "Call Ada @").unwrap();
    let schedule_offset = buffer_text(&registered.buffer()).find('@').unwrap() as i32;
    assert!(
        registered
            .buffer()
            .iter_at_offset(schedule_offset)
            .tags()
            .iter()
            .any(|tag| tag.name().as_deref() == Some("registered-schedule"))
    );
    let controls = find_css_class(
        registered.parent().unwrap().upcast_ref(),
        "canvas-entry-controls",
    )
    .unwrap();
    assert!(!controls.can_target());
    registered.grab_focus();
    drain_main_context();
    assert!(controls.can_target());

    let old_registered_text = buffer_text(&registered.buffer()).to_string();
    let old_suffix = &old_registered_text[old_registered_text.find('@').unwrap()..];
    registered
        .buffer()
        .set_text(&format!("Call Ada changed {old_suffix}"));
    gtk::prelude::WidgetExt::activate_action(
        window.widget(),
        "win.snooze",
        Some(&reminders[0].id.to_string().to_variant()),
    )
    .unwrap();
    drain_main_context();
    let snoozed = repository.get(reminders[0].id).unwrap();
    assert_eq!(snoozed.due_at, now + Duration::minutes(10));
    let registered =
        find_text_view_containing(window.widget().upcast_ref(), "Call Ada changed @").unwrap();
    let snoozed_text = buffer_text(&registered.buffer()).to_string();
    assert!(!snoozed_text.ends_with(old_suffix));
    assert_eq!(
        repository
            .list_canvas_entries()
            .unwrap()
            .into_iter()
            .find(|entry| entry.reminder_id == Some(reminders[0].id))
            .unwrap()
            .working_text
            .as_deref(),
        Some(snoozed_text.as_str())
    );

    let scheduled_entry = repository
        .list_canvas_entries()
        .unwrap()
        .into_iter()
        .find(|entry| entry.reminder_id.is_some())
        .unwrap();
    window.show_reminder(reminders[0].id);
    drain_main_context();
    let registered =
        find_text_view_containing(window.widget().upcast_ref(), "Call Ada changed @").unwrap();
    assert!(window_focuses(window.widget(), registered.upcast_ref()));
    gtk::prelude::WidgetExt::activate_action(
        window.widget(),
        "win.custom-time",
        Some(&scheduled_entry.id.to_string().to_variant()),
    )
    .unwrap();
    drain_main_context();
    let picker = find_calendar_popover(window.widget().upcast_ref()).expect("schedule picker");
    assert!(picker.has_css_class("reminder-picker"));
    assert!(!picker.has_css_class("menu"));
    let calendar = find_descendant::<gtk::Calendar>(picker.upcast_ref()).unwrap();
    assert!(calendar.has_css_class("reminder-calendar"));
    assert!(
        picker.pointing_to().0,
        "the picker must point to its source text"
    );
    for label in ["Today", "Tomorrow", "Time", "Cancel", "Apply"] {
        assert!(
            find_label_exact(picker.upcast_ref(), label).is_some(),
            "missing picker control {label}"
        );
    }
    let tomorrow = now.with_timezone(&Local).date_naive().succ_opt().unwrap();
    find_button_with_label(picker.upcast_ref(), "Tomorrow")
        .unwrap()
        .emit_clicked();
    drain_main_context();
    let selected = calendar.date();
    assert_eq!(
        (selected.year(), selected.month(), selected.day_of_month()),
        (
            tomorrow.year(),
            tomorrow.month() as i32,
            tomorrow.day() as i32
        )
    );
    let registered_text = buffer_text(&registered.buffer()).to_string();
    find_button_with_label(picker.upcast_ref(), "Cancel")
        .unwrap()
        .emit_clicked();
    drain_main_context();
    assert_eq!(buffer_text(&registered.buffer()), registered_text);
    assert!(
        picker.parent().is_none(),
        "a canceled picker must detach from its editor"
    );
    assert!(
        find_calendar_popover(window.widget().upcast_ref()).is_none(),
        "a canceled picker must leave the window hierarchy"
    );
    window.refresh();
    drain_main_context();
    assert_eq!(buffer_text(&registered.buffer()), registered_text);

    gtk::prelude::WidgetExt::activate_action(
        window.widget(),
        "win.custom-time",
        Some(&scheduled_entry.id.to_string().to_variant()),
    )
    .unwrap();
    drain_main_context();
    let picker = find_calendar_popover(window.widget().upcast_ref()).unwrap();
    let calendar = find_descendant::<gtk::Calendar>(picker.upcast_ref()).unwrap();
    let yesterday = now.with_timezone(&Local).date_naive().pred_opt().unwrap();
    calendar.set_date(
        &glib::DateTime::new(
            &glib::TimeZone::local(),
            yesterday.year(),
            yesterday.month() as i32,
            yesterday.day() as i32,
            12,
            0,
            0.0,
        )
        .unwrap(),
    );
    let due_before_invalid_apply = repository.get(reminders[0].id).unwrap().due_at;
    find_button_with_label(picker.upcast_ref(), "Apply")
        .unwrap()
        .emit_clicked();
    drain_main_context();
    assert!(find_label_containing(picker.upcast_ref(), "future").is_some());
    assert_eq!(
        repository.get(reminders[0].id).unwrap().due_at,
        due_before_invalid_apply
    );
    find_button_with_label(picker.upcast_ref(), "Tomorrow")
        .unwrap()
        .emit_clicked();
    find_button_with_label(picker.upcast_ref(), "Apply")
        .unwrap()
        .emit_clicked();
    drain_main_context();
    assert_eq!(
        repository
            .get(reminders[0].id)
            .unwrap()
            .due_at
            .with_timezone(&Local)
            .date_naive(),
        tomorrow
    );
    assert!(
        picker.parent().is_none(),
        "an applied picker must detach from its editor"
    );
    assert!(
        find_calendar_popover(window.widget().upcast_ref()).is_none(),
        "an applied picker must leave the window hierarchy"
    );

    gtk::prelude::WidgetExt::activate_action(window.widget(), "win.show-active-list", None)
        .unwrap();
    drain_main_context();
    let navigation = find_descendant::<adw::NavigationView>(window.widget().upcast_ref()).unwrap();
    assert_eq!(
        navigation.visible_page_tag().as_deref(),
        Some("active-list")
    );
    assert!(find_action_row(window.widget().upcast_ref(), "Call Ada changed").is_some());
    assert!(navigation.pop());
    gtk::prelude::WidgetExt::activate_action(window.widget(), "win.show-history", None).unwrap();
    drain_main_context();
    assert_eq!(navigation.visible_page_tag().as_deref(), Some("history"));
}

fn find_text_view_containing(root: &gtk::Widget, needle: &str) -> Option<gtk::TextView> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(view) = widget.clone().downcast::<gtk::TextView>()
            && buffer_text(&view.buffer()).contains(needle)
        {
            return Some(view);
        }
        if let Some(found) = find_text_view_containing(&widget, needle) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_action_row(root: &gtk::Widget, title: &str) -> Option<adw::ActionRow> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(row) = widget.clone().downcast::<adw::ActionRow>()
            && row.title() == title
        {
            return Some(row);
        }
        if let Some(found) = find_action_row(&widget, title) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_label_containing(root: &gtk::Widget, needle: &str) -> Option<gtk::Label> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(label) = widget.clone().downcast::<gtk::Label>()
            && label.label().contains(needle)
        {
            return Some(label);
        }
        if let Some(found) = find_label_containing(&widget, needle) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_label_exact(root: &gtk::Widget, expected: &str) -> Option<gtk::Label> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(label) = widget.clone().downcast::<gtk::Label>()
            && label.label() == expected
        {
            return Some(label);
        }
        if let Some(found) = find_label_exact(&widget, expected) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_icon_button(root: &gtk::Widget, icon_name: &str) -> Option<gtk::Button> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>()
            && button.icon_name().as_deref() == Some(icon_name)
        {
            return Some(button);
        }
        if let Some(found) = find_icon_button(&widget, icon_name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_button_with_label(root: &gtk::Widget, expected: &str) -> Option<gtk::Button> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>()
            && button.label().as_deref() == Some(expected)
        {
            return Some(button);
        }
        if let Some(found) = find_button_with_label(&widget, expected) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_css_class(root: &gtk::Widget, css_class: &str) -> Option<gtk::Widget> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if widget.has_css_class(css_class) {
            return Some(widget);
        }
        if let Some(found) = find_css_class(&widget, css_class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_all_css(root: &gtk::Widget, css_class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut child = root.first_child();
    while let Some(widget) = child {
        if widget.has_css_class(css_class) {
            found.push(widget.clone());
        }
        found.extend(find_all_css(&widget, css_class));
        child = widget.next_sibling();
    }
    found
}

fn find_calendar_popover(root: &gtk::Widget) -> Option<gtk::Popover> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(popover) = widget.clone().downcast::<gtk::Popover>()
            && find_descendant::<gtk::Calendar>(popover.upcast_ref()).is_some()
        {
            return Some(popover);
        }
        if let Some(found) = find_calendar_popover(&widget) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn buffer_text(buffer: &gtk::TextBuffer) -> glib::GString {
    buffer.text(&buffer.start_iter(), &buffer.end_iter(), false)
}

fn window_focuses(window: &adw::ApplicationWindow, widget: &gtk::Widget) -> bool {
    gtk::prelude::GtkWindowExt::focus(window)
        .as_ref()
        .is_some_and(|focused| focused == widget)
}

fn drain_main_context() {
    while glib::MainContext::default().iteration(false) {}
}

fn wait_until(predicate: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while !predicate() && std::time::Instant::now() < deadline {
        drain_main_context();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn find_descendant<T>(root: &gtk::Widget) -> Option<T>
where
    T: glib::object::IsA<gtk::Widget> + glib::types::StaticType,
{
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Ok(found) = widget.clone().downcast::<T>() {
            return Some(found);
        }
        if let Some(found) = find_descendant::<T>(&widget) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}
