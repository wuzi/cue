mod composer;
mod rows;
mod window;

use adw::prelude::*;
use thiserror::Error;

use crate::app::GETTEXT_PACKAGE;

pub use window::MainWindow;

pub struct WindowWidgets {
    pub window: adw::ApplicationWindow,
    pub toast_overlay: adw::ToastOverlay,
    pub navigation_view: adw::NavigationView,
    pub menu_button: gtk::MenuButton,
    pub composer_card: gtk::Box,
    pub composer_input: gtk::TextView,
    pub composer_placeholder: gtk::Label,
    pub add_button: gtk::Button,
    pub preview_button: gtk::Button,
    pub schedule_preview: gtk::Label,
    pub composer_error: gtk::Label,
    pub reminders_content: gtk::Stack,
    pub reminders_scroller: gtk::ScrolledWindow,
    pub reminders_empty: gtk::Box,
    pub composer_example_tomorrow: gtk::Label,
    pub composer_example_relative: gtk::Label,
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
        navigation_view: object(&builder, "navigation_view")?,
        menu_button: object(&builder, "menu_button")?,
        composer_card: object(&builder, "composer_card")?,
        composer_input: object(&builder, "composer_input")?,
        composer_placeholder: object(&builder, "composer_placeholder")?,
        add_button: object(&builder, "add_button")?,
        preview_button: object(&builder, "preview_button")?,
        schedule_preview: object(&builder, "schedule_preview")?,
        composer_error: object(&builder, "composer_error")?,
        reminders_content: object(&builder, "reminders_content")?,
        reminders_scroller: object(&builder, "reminders_scroller")?,
        reminders_empty: object(&builder, "reminders_empty")?,
        composer_example_tomorrow: object(&builder, "composer_example_tomorrow")?,
        composer_example_relative: object(&builder, "composer_example_relative")?,
        active_groups: object(&builder, "active_groups")?,
        history_list: object(&builder, "history_list")?,
        history_empty: object(&builder, "history_empty")?,
        clear_history_button: object(&builder, "clear_history_button")?,
    };

    // These remain canonical English examples for the English-only v1 grammar.
    widgets
        .composer_example_tomorrow
        .set_label("Call Ada @tomorrow 9am");
    widgets
        .composer_example_relative
        .set_label("Take a break @in 30 minutes");

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
    InvalidTemplate(#[from] glib::Error),
}
