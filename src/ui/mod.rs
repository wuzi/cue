mod window;

use adw::prelude::*;
use glib::value::ToValue;
use thiserror::Error;

use crate::app::GETTEXT_PACKAGE;

pub use window::MainWindow;

pub struct WindowWidgets {
    pub window: adw::ApplicationWindow,
    pub toast_overlay: adw::ToastOverlay,
    pub view_stack: adw::ViewStack,
    pub header_switcher: adw::ViewSwitcher,
    pub bottom_switcher: adw::ViewSwitcherBar,
    pub menu_button: gtk::MenuButton,
    pub message_entry: adw::EntryRow,
    pub add_button: gtk::Button,
    pub when_row: adw::ActionRow,
    pub composer_error: gtk::Label,
    pub reminders_content: gtk::Stack,
    pub reminders_scroller: gtk::ScrolledWindow,
    pub active_groups: gtk::Box,
    pub history_list: gtk::Box,
    pub history_empty: adw::StatusPage,
    pub clear_history_button: gtk::Button,
}

pub fn build_window(application: &adw::Application) -> Result<WindowWidgets, UiBuildError> {
    let builder = gtk::Builder::new();
    builder.set_translation_domain(Some(GETTEXT_PACKAGE));
    builder.add_from_resource("/io/github/wuzi/RemindMe/main-window.ui")?;
    let window = object::<adw::ApplicationWindow>(&builder, "main_window")?;
    window.set_application(Some(application));

    let widgets = WindowWidgets {
        window,
        toast_overlay: object(&builder, "toast_overlay")?,
        view_stack: object(&builder, "view_stack")?,
        header_switcher: object(&builder, "header_switcher")?,
        bottom_switcher: object(&builder, "bottom_switcher")?,
        menu_button: object(&builder, "menu_button")?,
        message_entry: object(&builder, "message_entry")?,
        add_button: object(&builder, "add_button")?,
        when_row: object(&builder, "when_row")?,
        composer_error: object(&builder, "composer_error")?,
        reminders_content: object(&builder, "reminders_content")?,
        reminders_scroller: object(&builder, "reminders_scroller")?,
        active_groups: object(&builder, "active_groups")?,
        history_list: object(&builder, "history_list")?,
        history_empty: object(&builder, "history_empty")?,
        clear_history_button: object(&builder, "clear_history_button")?,
    };

    let condition = adw::BreakpointCondition::parse("max-width: 550sp")?;
    let breakpoint = adw::Breakpoint::new(condition);
    breakpoint.add_setter(&widgets.header_switcher, "visible", Some(&false.to_value()));
    breakpoint.add_setter(&widgets.bottom_switcher, "reveal", Some(&true.to_value()));
    widgets.window.add_breakpoint(breakpoint);

    Ok(widgets)
}

fn object<T>(builder: &gtk::Builder, id: &'static str) -> Result<T, UiBuildError>
where
    T: glib::object::IsA<glib::Object>,
{
    builder.object(id).ok_or(UiBuildError::MissingObject(id))
}

#[derive(Debug, Error)]
pub enum UiBuildError {
    #[error("the UI template is missing object {0}")]
    MissingObject(&'static str),
    #[error(transparent)]
    InvalidBreakpoint(#[from] glib::BoolError),
    #[error(transparent)]
    InvalidTemplate(#[from] glib::Error),
}
