mod canvas;
mod rows;
mod schedule_picker;
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
    pub active_list_button: gtk::Button,
    pub canvas_scroller: gtk::ScrolledWindow,
    pub canvas_entries: gtk::Box,
    pub active_content: gtk::Stack,
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
        active_list_button: object(&builder, "active_list_button")?,
        canvas_scroller: object(&builder, "canvas_scroller")?,
        canvas_entries: object(&builder, "canvas_entries")?,
        active_content: object(&builder, "active_content")?,
        active_groups: object(&builder, "active_groups")?,
        history_list: object(&builder, "history_list")?,
        history_empty: object(&builder, "history_empty")?,
        clear_history_button: object(&builder, "clear_history_button")?,
    };

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
