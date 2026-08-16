pub fn register() -> Result<(), glib::Error> {
    gio::resources_register_include!("cue.gresource")
}
