use std::ops::Range;

use adw::prelude::*;

use super::WindowWidgets;

pub struct SmartComposer {
    input: gtk::TextView,
    placeholder: gtk::Label,
    add_button: gtk::Button,
    preview: gtk::Label,
    preview_button: gtk::Button,
    error: gtk::Label,
    schedule_tag: gtk::TextTag,
}

impl SmartComposer {
    pub fn new(widgets: &WindowWidgets) -> Self {
        let schedule_tag = gtk::TextTag::builder().name("schedule").build();
        widgets
            .composer_input
            .buffer()
            .tag_table()
            .add(&schedule_tag);
        widgets
            .composer_input
            .buffer()
            .connect_insert_text(|buffer, position, text| {
                if !text.contains(['\n', '\r']) {
                    return;
                }
                buffer.stop_signal_emission_by_name("insert-text");
                let normalized = text.replace("\r\n", " ").replace(['\n', '\r'], " ");
                buffer.insert(position, &normalized);
            });
        update_accent(&schedule_tag, &adw::StyleManager::default());

        let tag = schedule_tag.clone();
        adw::StyleManager::default().connect_accent_color_notify(move |manager| {
            update_accent(&tag, manager);
        });
        let tag = schedule_tag.clone();
        adw::StyleManager::default().connect_dark_notify(move |manager| {
            update_accent(&tag, manager);
        });

        Self {
            input: widgets.composer_input.clone(),
            placeholder: widgets.composer_placeholder.clone(),
            add_button: widgets.add_button.clone(),
            preview: widgets.schedule_preview.clone(),
            preview_button: widgets.preview_button.clone(),
            error: widgets.composer_error.clone(),
            schedule_tag,
        }
    }

    pub fn text(&self) -> String {
        let buffer = self.input.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }

    pub fn clear(&self) {
        self.input.buffer().set_text("");
    }

    pub fn update_span(&self, input: &str, span: Option<Range<usize>>) {
        let buffer = self.input.buffer();
        let start = buffer.start_iter();
        let end = buffer.end_iter();
        buffer.remove_tag(&self.schedule_tag, &start, &end);
        if let Some(span) = span {
            let start_offset = input[..span.start].chars().count() as i32;
            let end_offset = input[..span.end].chars().count() as i32;
            buffer.apply_tag(
                &self.schedule_tag,
                &buffer.iter_at_offset(start_offset),
                &buffer.iter_at_offset(end_offset),
            );
        }
    }

    pub fn set_preview(&self, value: &str) {
        self.preview.set_label(value);
        self.preview_button.set_tooltip_text(Some(value));
        self.preview_button
            .update_property(&[gtk::accessible::Property::Description(value)]);
    }

    pub fn set_error(&self, error: Option<&str>) {
        if let Some(error) = error {
            self.error.set_label(error);
            self.error.set_visible(true);
            self.input
                .update_property(&[gtk::accessible::Property::Description(error)]);
        } else {
            self.error.set_visible(false);
            self.input.update_property(&[
                gtk::accessible::Property::Description(&gettextrs::gettext(
                    "Write what to remember, followed by an optional schedule beginning with at sign",
                )),
            ]);
        }
    }

    pub fn set_can_submit(&self, can_submit: bool) {
        self.add_button.set_sensitive(can_submit);
    }

    pub fn update_placeholder(&self) {
        self.placeholder.set_visible(self.text().is_empty());
    }
}

fn update_accent(tag: &gtk::TextTag, manager: &adw::StyleManager) {
    let color = manager.accent_color().to_standalone_rgba(manager.is_dark());
    tag.set_foreground_rgba(Some(&color));
}
