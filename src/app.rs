#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BackgroundHold {
    held: bool,
}

impl BackgroundHold {
    pub fn update(&mut self, should_hold: bool) -> Option<HoldChange> {
        if self.held == should_hold {
            return None;
        }
        self.held = should_hold;
        Some(if should_hold {
            HoldChange::Hold
        } else {
            HoldChange::Release
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldChange {
    Hold,
    Release,
}

pub const DATA_DIRECTORY_NAME: &str = "cue";

#[derive(Debug, Default)]
pub struct RuntimeWarningState {
    reported: HashSet<String>,
    pending: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeWarningAction {
    Ignore,
    LogOnly,
    LogAndShow,
}

impl RuntimeWarningState {
    pub fn report_notification_warning(
        &mut self,
        message: &str,
        has_window: bool,
    ) -> RuntimeWarningAction {
        if !self.reported.insert(message.to_owned()) {
            return RuntimeWarningAction::Ignore;
        }
        if has_window {
            RuntimeWarningAction::LogAndShow
        } else {
            self.pending.push(message.to_owned());
            RuntimeWarningAction::LogOnly
        }
    }

    pub fn report_notification_error(
        &mut self,
        error: &NotificationError,
        has_window: bool,
    ) -> (RuntimeWarningAction, String) {
        let (action, message) = match error {
            NotificationError::MissingDesktopEntry { .. } => {
                let message = gettextrs::gettext(DESKTOP_METADATA_WARNING);
                (
                    self.report_notification_warning(&message, has_window),
                    message,
                )
            }
            _ => {
                let message = error.to_string();
                (self.report_runtime_error(&message, has_window), message)
            }
        };
        (action, message)
    }

    pub fn report_runtime_error(
        &mut self,
        _message: &str,
        has_window: bool,
    ) -> RuntimeWarningAction {
        if has_window {
            RuntimeWarningAction::LogAndShow
        } else {
            RuntimeWarningAction::LogOnly
        }
    }

    pub fn take_pending(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending)
    }
}

pub fn run() -> glib::ExitCode {
    initialize_gettext();
    initialize_logging();
    if let Err(error) = resources::register() {
        error!(%error, "failed to register application resources");
        return glib::ExitCode::FAILURE;
    }

    let application = adw::Application::builder()
        .application_id(APPLICATION_ID)
        .build();
    let runtime = Rc::new(RefCell::new(None::<Result<Rc<AppRuntime>, String>>));

    let startup_runtime = runtime.clone();
    application.connect_startup(move |application| {
        install_css();
        let value = AppRuntime::new(application).map_err(|error| error.to_string());
        *startup_runtime.borrow_mut() = Some(value);
    });

    application.connect_activate(move |application| match runtime.borrow().as_ref() {
        Some(Ok(runtime)) => runtime.show_window(),
        Some(Err(message)) => show_startup_error(application, message),
        None => show_startup_error(
            application,
            &gettextrs::gettext("The application did not finish starting."),
        ),
    });

    application.run()
}

struct AppRuntime {
    application: adw::Application,
    service: Rc<ReminderService>,
    window: RefCell<Option<Rc<MainWindow>>>,
    timer: RefCell<Option<glib::SourceId>>,
    background_hold: RefCell<BackgroundHold>,
    hold_guard: RefCell<Option<gio::ApplicationHoldGuard>>,
    runtime_warnings: RefCell<RuntimeWarningState>,
}

impl AppRuntime {
    fn new(application: &adw::Application) -> Result<Rc<Self>, AppStartupError> {
        let data_directory = glib::user_data_dir().join(DATA_DIRECTORY_NAME);
        fs::create_dir_all(&data_directory)?;
        let repository = Rc::new(SqliteReminderRepository::open(
            data_directory.join("reminders.db"),
        )?);
        let clock = Rc::new(SystemClock);
        let notifier = Rc::new(GioReminderNotifier::new(
            application.clone().upcast::<gio::Application>(),
        ));
        let availability_error = notifier.availability().err();
        let service = Rc::new(ReminderService::new(repository, clock, notifier));
        let runtime = Rc::new(Self {
            application: application.clone(),
            service,
            window: RefCell::new(None),
            timer: RefCell::new(None),
            background_hold: RefCell::new(BackgroundHold::default()),
            hold_guard: RefCell::new(None),
            runtime_warnings: RefCell::new(RuntimeWarningState::default()),
        });
        runtime.install_actions();
        if let Some(error) = availability_error {
            runtime.report_notification_error(&error);
        }
        runtime.refresh_scheduler();
        Ok(runtime)
    }

    fn install_actions(self: &Rc<Self>) {
        let done = gio::SimpleAction::new("done", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        done.connect_activate(move |_, target| {
            let Some(runtime) = weak.upgrade() else {
                return;
            };
            let Some(target) = target.and_then(String::from_variant) else {
                return;
            };
            match runtime.service.complete_target(&target) {
                Ok(ActionOutcome::Applied) => runtime.reconcile_after_mutation(),
                Ok(ActionOutcome::Ignored) => {}
                Err(error) => runtime.report_service_error(&error),
            }
        });
        self.application.add_action(&done);

        let snooze = gio::SimpleAction::new("snooze", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        snooze.connect_activate(move |_, target| {
            let Some(runtime) = weak.upgrade() else {
                return;
            };
            let Some(target) = target.and_then(String::from_variant) else {
                return;
            };
            match runtime.service.snooze_target(&target) {
                Ok(ActionOutcome::Applied) => runtime.reconcile_after_mutation(),
                Ok(ActionOutcome::Ignored) => {}
                Err(error) => runtime.report_service_error(&error),
            }
        });
        self.application.add_action(&snooze);

        let show = gio::SimpleAction::new("show-reminder", Some(&String::static_variant_type()));
        let weak = Rc::downgrade(self);
        show.connect_activate(move |_, target| {
            let Some(runtime) = weak.upgrade() else {
                return;
            };
            let Some(target) = target.and_then(String::from_variant) else {
                return;
            };
            match runtime.service.resolve_active_target(&target) {
                Ok(Some(id)) => runtime.show_reminder(id),
                Ok(None) => {}
                Err(error) => runtime.report_service_error(&error),
            }
        });
        self.application.add_action(&show);

        let quit = gio::SimpleAction::new("quit", None);
        let weak = Rc::downgrade(self);
        quit.connect_activate(move |_, _| {
            let Some(runtime) = weak.upgrade() else {
                return;
            };
            runtime.show_window();
            if let Some(window) = runtime.window.borrow().as_ref() {
                window.confirm_quit(&runtime.application);
            }
        });
        self.application.add_action(&quit);
        self.application
            .set_accels_for_action("app.quit", &["<primary>q"]);

        let about = gio::SimpleAction::new("about", None);
        let weak = Rc::downgrade(self);
        about.connect_activate(move |_, _| {
            if let Some(runtime) = weak.upgrade() {
                runtime.show_about();
            }
        });
        self.application.add_action(&about);
    }

    fn show_window(self: &Rc<Self>) {
        if let Some(window) = self.window.borrow().as_ref() {
            window.present();
            return;
        }

        let mutation_runtime = Rc::downgrade(self);
        let closed_runtime = Rc::downgrade(self);
        match MainWindow::new(
            &self.application,
            self.service.clone(),
            move || {
                if let Some(runtime) = mutation_runtime.upgrade() {
                    runtime.reconcile_after_mutation();
                }
            },
            move || {
                if let Some(runtime) = closed_runtime.upgrade() {
                    runtime.window.borrow_mut().take();
                }
            },
        ) {
            Ok(window) => {
                window.present();
                self.window.borrow_mut().replace(window);
                self.show_pending_runtime_warnings();
            }
            Err(error) => show_startup_error(&self.application, &error.to_string()),
        }
    }

    fn show_reminder(self: &Rc<Self>, id: uuid::Uuid) {
        self.show_window();
        if let Some(window) = self.window.borrow().as_ref() {
            window.show_reminder(id);
        }
    }

    fn show_about(self: &Rc<Self>) {
        self.show_window();
        let dialog = adw::AboutDialog::new();
        dialog.set_application_name("Cue");
        dialog.set_application_icon(APPLICATION_ID);
        dialog.set_developer_name("wuzi");
        dialog.set_version(env!("CARGO_PKG_VERSION"));
        dialog.set_license_type(gtk::License::Gpl30);
        dialog.set_comments(&gettextrs::gettext(
            "Create quick notes that remind you at the right time.",
        ));
        if let Some(window) = self.window.borrow().as_ref() {
            dialog.present(Some(window.widget()));
        }
    }

    fn refresh_scheduler(self: &Rc<Self>) {
        let refresh_succeeded = match self.service.refresh() {
            Ok(_) => true,
            Err(error) => {
                self.report_service_error(&error);
                false
            }
        };
        if let Some(window) = self.window.borrow().as_ref() {
            window.refresh();
        }
        self.update_background_hold();
        self.reschedule(refresh_succeeded);
    }

    fn reconcile_after_mutation(self: &Rc<Self>) {
        if let Some(window) = self.window.borrow().as_ref() {
            window.refresh();
        }
        self.update_background_hold();
        self.reschedule(true);
    }

    fn update_background_hold(&self) {
        let should_hold = match self.service.should_hold_background() {
            Ok(value) => value,
            Err(error) => {
                self.report_service_error(&error);
                true
            }
        };
        match self.background_hold.borrow_mut().update(should_hold) {
            Some(HoldChange::Hold) => {
                let guard = gio::prelude::ApplicationExtManual::hold(&self.application);
                self.hold_guard.borrow_mut().replace(guard);
            }
            Some(HoldChange::Release) => {
                self.hold_guard.borrow_mut().take();
            }
            None => {}
        }
    }

    fn reschedule(self: &Rc<Self>, refresh_succeeded: bool) {
        if let Some(source) = self.timer.borrow_mut().take() {
            source.remove();
        }
        let next_due = if refresh_succeeded {
            match self.service.next_due() {
                Ok(next_due) => next_due,
                Err(error) => {
                    self.report_service_error(&error);
                    None
                }
            }
        } else {
            None
        };
        let delay = refresh_wakeup_delay(refresh_succeeded, Utc::now(), next_due)
            .max(Duration::from_millis(50));
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(delay, move || {
            if let Some(runtime) = weak.upgrade() {
                runtime.timer.borrow_mut().take();
                runtime.refresh_scheduler();
            }
        });
        self.timer.borrow_mut().replace(source);
    }

    fn report_runtime_error(&self, message: &str) {
        let has_window = self.window.borrow().is_some();
        match self
            .runtime_warnings
            .borrow_mut()
            .report_runtime_error(message, has_window)
        {
            RuntimeWarningAction::Ignore => {}
            RuntimeWarningAction::LogOnly => warn!(message, "reminder operation failed"),
            RuntimeWarningAction::LogAndShow => {
                warn!(message, "reminder operation failed");
                if let Some(window) = self.window.borrow().as_ref() {
                    window.show_error_message(message);
                }
            }
        }
    }

    fn report_notification_error(&self, error: &NotificationError) {
        let has_window = self.window.borrow().is_some();
        let (action, message) = self
            .runtime_warnings
            .borrow_mut()
            .report_notification_error(error, has_window);
        match action {
            RuntimeWarningAction::Ignore => {}
            RuntimeWarningAction::LogOnly => warn!(message, "reminder operation failed"),
            RuntimeWarningAction::LogAndShow => {
                warn!(message, "reminder operation failed");
                if let Some(window) = self.window.borrow().as_ref() {
                    window.show_error_message(&message);
                }
            }
        }
    }

    fn report_service_error(&self, error: &ServiceError) {
        match error {
            ServiceError::Scheduler(SchedulerError::Notification(notification_error)) => {
                self.report_notification_error(notification_error)
            }
            _ => self.report_runtime_error(&error.to_string()),
        }
    }

    fn show_pending_runtime_warnings(&self) {
        let pending = self.runtime_warnings.borrow_mut().take_pending();
        if let Some(window) = self.window.borrow().as_ref() {
            for message in pending {
                window.show_error_message(&message);
            }
        }
    }
}

fn initialize_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

fn initialize_gettext() {
    // This runs before GTK, logging, or any application worker can create threads.
    unsafe {
        gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    }
    let locale_directory = option_env!("LOCALEDIR").unwrap_or("/usr/share/locale");
    if let Err(error) = gettextrs::bindtextdomain(GETTEXT_PACKAGE, locale_directory) {
        warn!(%error, "failed to bind gettext domain");
    }
    if let Err(error) = gettextrs::bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8") {
        warn!(%error, "failed to select the gettext encoding");
    }
    if let Err(error) = gettextrs::textdomain(GETTEXT_PACKAGE) {
        warn!(%error, "failed to select gettext domain");
    }
}

fn install_css() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_resource("/io/github/wuzi/Cue/style.css");
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn show_startup_error(application: &adw::Application, message: &str) {
    let window = adw::ApplicationWindow::builder()
        .application(application)
        .title("Cue")
        .default_width(520)
        .default_height(420)
        .build();
    let page = adw::StatusPage::builder()
        .icon_name("dialog-error-symbolic")
        .title(gettextrs::gettext("Reminders are unavailable"))
        .description(message)
        .build();
    window.set_content(Some(&page));
    window.present();
}

#[derive(Debug, Error)]
enum AppStartupError {
    #[error("Could not prepare the reminder data directory: {0}")]
    DataDirectory(#[from] std::io::Error),
    #[error("Could not open the reminder database: {0}")]
    Database(#[from] crate::repository::RepositoryError),
}
use std::{cell::RefCell, collections::HashSet, fs, rc::Rc, time::Duration};

use adw::prelude::*;
use chrono::Utc;
use gio::prelude::{ActionMapExt, ApplicationExt};
use glib::variant::{FromVariant, StaticVariantType};
use thiserror::Error;
use tracing::{error, warn};
use tracing_subscriber::EnvFilter;

use crate::{
    notifications::GioReminderNotifier,
    repository::SqliteReminderRepository,
    resources,
    scheduler::{
        NotificationError, ReminderNotifier, SchedulerError, SystemClock, refresh_wakeup_delay,
    },
    service::{ActionOutcome, ReminderService, ServiceError},
    ui::MainWindow,
};

pub const APPLICATION_ID: &str = "io.github.wuzi.Cue";
pub(crate) const GETTEXT_PACKAGE: &str = "cue";

const DESKTOP_METADATA_WARNING: &str =
    "Notifications are unavailable in this development run. Install Cue to receive reminders.";
