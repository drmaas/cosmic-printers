// SPDX-License-Identifier: GPL-3.0-only

use cosmic::iced::platform_specific::shell::wayland::commands::popup::{destroy_popup, get_popup};
use cosmic::iced::window::Id;
use cosmic::iced::{Alignment, Length, Limits, Subscription, Task};
use cosmic::prelude::*;
use cosmic::widget;
use futures::SinkExt;

use crate::config::{self, Config, ShowIcon};
use crate::cups::{self, ApplyResult, CupsSignal, Queue, QueueError, Tools};
use crate::fl;
use crate::status::{
    self, ICON_RUNNING, JOB_STATE_PROCESSING, PanelStatus, StatusInput, Tooltip, WarningKind,
    icon_visible, panel_status, printer_icon, row_notice, truncate_chars,
};

pub const APP_ID: &str = "io.github.drmaas.CosmicPrinters";

pub struct AppModel {
    core: cosmic::Core,
    popup: Option<Id>,
    config: Config,
    queue: Queue,
    tools: Tools,
    pending: Option<Pending>,
}

#[derive(Debug, Clone)]
enum Pending {
    CancelJob {
        printer: String,
        job_id: u32,
        name: String,
    },
    CancelAll {
        printer: String,
        job_ids: Vec<u32>,
    },
}

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    Surface(cosmic::surface::Action<Message>),
    UpdateConfig(Config),
    ShowIcon(usize),
    Snapshot(Queue),
    Cups(CupsSignal),
    Tools(Tools),
    OpenPrinterSettings(Option<String>),
    OpenProperties(String),
    AskCancelJob {
        printer: String,
        job_id: u32,
        name: String,
    },
    AskCancelAll {
        printer: String,
        job_ids: Vec<u32>,
    },
    ConfirmPending,
    DismissDialog,
    SendToFront {
        printer: String,
        job_id: u32,
    },
    PropertiesOpened,
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let app = Self {
            core,
            popup: None,
            config: config::load(),
            queue: Queue {
                printers: indexmap::IndexMap::new(),
                error: None,
            },
            tools: Tools {
                config_tool: false,
                properties: false,
            },
            pending: None,
        };
        (
            app,
            Task::batch([
                cosmic::task::future(async { Message::Snapshot(cups::bootstrap().await) }),
                cosmic::task::future(async { Message::Tools(cups::detect_tools().await) }),
            ]),
        )
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let status = self.panel_status();
        if !icon_visible(
            self.config.show_icon,
            self.queue.printers.len(),
            status.job_count,
        ) {
            return widget::Space::new()
                .width(Length::Fixed(0.0))
                .height(Length::Fixed(0.0))
                .into();
        }

        let icon_size = self.core.applet.suggested_size(true).0;
        let (major, minor) = self.core.applet.suggested_padding(true);
        let (horizontal, vertical) = if self.core.applet.is_horizontal() {
            (major, minor)
        } else {
            (minor, major)
        };
        let icon = widget::icon::from_name(status.icon)
            .symbolic(true)
            .size(icon_size);
        let content: Element<'_, Message> = if status.job_count > 0 {
            let count = widget::text(status.job_count.to_string());
            if self.core.applet.is_horizontal() {
                widget::Row::new()
                    .push(icon)
                    .push(count)
                    .spacing(4)
                    .align_y(Alignment::Center)
                    .into()
            } else {
                widget::Column::new()
                    .push(icon)
                    .push(count)
                    .spacing(2)
                    .align_x(Alignment::Center)
                    .into()
            }
        } else {
            icon.into()
        };

        let button = widget::button::custom(content)
            .padding([vertical as f32, horizontal as f32])
            .on_press(Message::TogglePopup)
            .class(cosmic::theme::Button::AppletIcon);
        self.core
            .applet
            .applet_tooltip(
                button,
                self.tooltip_text(&status),
                self.popup.is_some(),
                Message::Surface,
                self.core.main_window_id(),
            )
            .into()
    }

    fn view_window(&self, id: Id) -> Element<'_, Self::Message> {
        if self.popup != Some(id) {
            return widget::text("").into();
        }
        let content = if let Some(pending) = &self.pending {
            self.confirm_dialog(pending)
        } else {
            self.popup_list(id)
        };
        self.core.applet.popup_container(content).into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch(vec![
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
            cups_subscription(),
        ])
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::TogglePopup => return self.toggle_popup(),
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    self.pending = None;
                }
            }
            Message::Surface(action) => {
                return Task::done(cosmic::action::surface(action));
            }
            Message::UpdateConfig(config) => self.config = config,
            Message::ShowIcon(index) => {
                self.config.show_icon = ShowIcon::from_index(index);
                config::save(&self.config);
            }
            Message::Snapshot(queue) => self.queue = queue,
            Message::Cups(signal) => {
                if self.queue.handle_signal(signal) == ApplyResult::Reset {
                    return cosmic::task::future(async {
                        Message::Snapshot(cups::bootstrap().await)
                    });
                }
            }
            Message::Tools(tools) => self.tools = tools,
            Message::OpenPrinterSettings(printer) => {
                if let Err(error) = cups::open_config_tool(printer.as_deref()) {
                    eprintln!("cosmic-printers: {error}");
                }
                return self.close_popup();
            }
            Message::OpenProperties(printer) => {
                let close = self.close_popup();
                let open = cosmic::task::future(async move {
                    if let Err(error) = cups::open_properties(&printer).await {
                        eprintln!("cosmic-printers: {error}");
                    }
                    Message::PropertiesOpened
                });
                return Task::batch([close, open]);
            }
            Message::AskCancelJob {
                printer,
                job_id,
                name,
            } => {
                self.pending = Some(Pending::CancelJob {
                    printer,
                    job_id,
                    name,
                });
            }
            Message::AskCancelAll { printer, job_ids } => {
                self.pending = Some(Pending::CancelAll { printer, job_ids });
            }
            Message::ConfirmPending => {
                if let Some(pending) = self.pending.take() {
                    confirm(pending);
                }
            }
            Message::DismissDialog => self.pending = None,
            Message::SendToFront { printer, job_id } => {
                match cups::prioritize_job(&printer, job_id) {
                    Ok(()) => {
                        if let Some(item) = self.queue.printers.get_mut(&printer) {
                            cups::send_job_to_front(&mut item.jobs, job_id);
                        }
                    }
                    Err(error) => eprintln!("cosmic-printers: {error}"),
                }
            }
            Message::PropertiesOpened => {}
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

impl AppModel {
    fn panel_status(&self) -> PanelStatus {
        let inputs = self
            .queue
            .printers
            .values()
            .map(|printer| StatusInput {
                display_name: printer.display_name.clone(),
                state: printer.state,
                state_reasons: printer.state_reasons.clone(),
                accepting: printer.accepting,
                job_count: printer.jobs.len(),
            })
            .collect::<Vec<_>>();
        panel_status(&inputs)
    }

    fn tooltip_text(&self, status: &PanelStatus) -> String {
        match &status.tooltip {
            Tooltip::Printers => fl!("tooltip-printers"),
            Tooltip::Jobs(count) => fl!("job-count", count = count_arg(*count)),
            Tooltip::Warning { printer, kind } => {
                let message = warning_text(*kind);
                fl!(
                    "printer-warning",
                    printer = printer.as_str(),
                    message = message.as_str()
                )
            }
        }
    }

    fn toggle_popup(&mut self) -> Task<cosmic::Action<Message>> {
        if let Some(id) = self.popup.take() {
            self.pending = None;
            return destroy_popup(id);
        }
        let Some(parent) = self.core.main_window_id() else {
            return Task::none();
        };
        let id = Id::unique();
        self.popup = Some(id);
        let mut settings = self
            .core
            .applet
            .get_popup_settings(parent, id, None, None, None);
        settings.positioner.size_limits = Limits::NONE
            .min_width(300.0)
            .max_width(372.0)
            .min_height(1.0)
            .max_height(1080.0);
        get_popup(settings)
    }

    fn close_popup(&mut self) -> Task<cosmic::Action<Message>> {
        self.pending = None;
        match self.popup.take() {
            Some(id) => destroy_popup(id),
            None => Task::none(),
        }
    }

    fn confirm_dialog(&self, pending: &Pending) -> Element<'_, Message> {
        let (title, body) = match pending {
            Pending::CancelJob { name, .. } => (
                fl!("cancel-one-title"),
                fl!("cancel-one-body", name = name.as_str()),
            ),
            Pending::CancelAll { .. } => (fl!("cancel-all-title"), fl!("cancel-all-body")),
        };
        widget::dialog()
            .title(title)
            .body(body)
            .primary_action(
                widget::button::destructive(fl!("cancel-printing"))
                    .on_press(Message::ConfirmPending),
            )
            .secondary_action(
                widget::button::standard(fl!("keep")).on_press(Message::DismissDialog),
            )
            .into()
    }

    fn popup_list(&self, id: Id) -> Element<'_, Message> {
        let mut column = widget::Column::new()
            .spacing(12)
            .padding(16)
            .width(Length::Fill);
        if let Some(error) = self.queue.error {
            column = column.push(widget::text(queue_error_text(error)));
        }

        if self.queue.printers.is_empty() {
            column = column.push(self.printers_button(None));
        } else {
            let mut previous_had_jobs = false;
            for printer in self.queue.printers.values() {
                if previous_had_jobs || !printer.jobs.is_empty() {
                    column = column.push(widget::divider::horizontal::default());
                }
                previous_had_jobs = !printer.jobs.is_empty();
                column = column.push(self.printer_block(printer));
            }
            column = column.push(widget::divider::horizontal::default());
            column = column.push(self.printers_button(None));
        }

        let labels = vec![fl!("show-always"), fl!("show-printers"), fl!("show-jobs")];
        let show_icon = widget::settings::item(
            fl!("show-icon"),
            widget::dropdown::popup_dropdown(
                labels,
                Some(self.config.show_icon.index()),
                Message::ShowIcon,
                id,
                Message::Surface,
                |message| message,
            ),
        );
        column = column.push(show_icon);
        column.into()
    }

    fn printers_button(&self, printer: Option<String>) -> Element<'_, Message> {
        let message = self
            .tools
            .config_tool
            .then_some(Message::OpenPrinterSettings(printer));
        widget::button::text(fl!("printers"))
            .on_press_maybe(message)
            .width(Length::Fill)
            .into()
    }

    fn printer_block(&self, printer: &cups::Printer) -> Element<'_, Message> {
        let mut block = widget::Column::new().spacing(4).width(Length::Fill);
        let mut header = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .width(Length::Fill);
        let label = widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(
                widget::icon::from_name(printer_icon(printer.state, &printer.state_reasons))
                    .symbolic(true)
                    .size(16),
            )
            .push(widget::text(printer.display_name.clone()).width(Length::Fill));
        let open = self
            .tools
            .config_tool
            .then(|| Message::OpenPrinterSettings(Some(printer.name.clone())));
        header = header.push(
            widget::button::custom(label)
                .on_press_maybe(open)
                .width(Length::Fill),
        );
        if self.tools.properties {
            header = header.push(action_button(
                "emblem-system-symbolic",
                fl!("show-properties"),
                Message::OpenProperties(printer.name.clone()),
            ));
        }
        block = block.push(header);

        if let Some(kind) = row_notice(&printer.state_reasons, printer.accepting) {
            block = block.push(widget::text(warning_text(kind)).size(12));
        }

        let first_processing = printer
            .jobs
            .first()
            .is_some_and(|(_, job)| job.state == JOB_STATE_PROCESSING);
        for (index, job) in printer.jobs.values().enumerate() {
            block = block.push(self.job_row(printer, job, index, first_processing));
        }
        if printer.jobs.len() > 1 {
            let job_ids = printer.jobs.keys().copied().collect::<Vec<_>>();
            block = block.push(self.cancel_all_row(&printer.name, job_ids));
        }
        block.into()
    }

    fn job_row(
        &self,
        printer: &cups::Printer,
        job: &cups::Job,
        index: usize,
        first_processing: bool,
    ) -> Element<'_, Message> {
        let title = if job.name.is_empty() {
            fl!("unknown")
        } else {
            truncate_chars(&job.name, 30)
        };
        let mut title_row = widget::Row::new().spacing(8).align_y(Alignment::Center);
        if job.state == JOB_STATE_PROCESSING {
            title_row = title_row.push(
                widget::icon::from_name(ICON_RUNNING)
                    .symbolic(true)
                    .size(16),
            );
        }
        title_row = title_row
            .push(widget::text(title).width(Length::Fill))
            .push(widget::text(job.created_display.clone()).size(12));

        let mut actions = widget::Row::new().spacing(4);
        if status::can_send_to_front(index, first_processing) {
            actions = actions.push(action_button(
                "go-up-symbolic",
                fl!("send-to-front"),
                Message::SendToFront {
                    printer: printer.name.clone(),
                    job_id: job.id,
                },
            ));
        }
        let name = if job.name.is_empty() {
            fl!("unknown")
        } else {
            job.name.clone()
        };
        actions = actions.push(action_button(
            "edit-delete-symbolic",
            fl!("cancel-job"),
            Message::AskCancelJob {
                printer: printer.name.clone(),
                job_id: job.id,
                name,
            },
        ));

        widget::Column::new()
            .spacing(2)
            .push(title_row)
            .push(actions)
            .into()
    }

    fn cancel_all_row(&self, printer: &str, job_ids: Vec<u32>) -> Element<'_, Message> {
        widget::Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(widget::text(fl!("cancel-all")).width(Length::Fill))
            .push(action_button(
                "edit-delete-symbolic",
                fl!("cancel-all"),
                Message::AskCancelAll {
                    printer: printer.to_string(),
                    job_ids,
                },
            ))
            .into()
    }
}

fn confirm(pending: Pending) {
    let result = match pending {
        Pending::CancelJob {
            printer, job_id, ..
        } => cups::cancel_job(&printer, job_id),
        Pending::CancelAll { printer, job_ids } => {
            let mut last = Ok(());
            for job_id in job_ids {
                last = cups::cancel_job(&printer, job_id);
            }
            last
        }
    };
    if let Err(error) = result {
        eprintln!("cosmic-printers: {error}");
    }
}

fn action_button<'a>(
    icon_name: &'a str,
    tooltip: impl Into<std::borrow::Cow<'a, str>>,
    message: Message,
) -> Element<'a, Message> {
    widget::button::icon(widget::icon::from_name(icon_name).symbolic(true).size(16))
        .tooltip(tooltip)
        .on_press(message)
        .into()
}

fn warning_text(kind: WarningKind) -> String {
    match kind {
        WarningKind::OutOfPaper => fl!("out-of-paper"),
        WarningKind::PaperJam => fl!("paper-jam"),
        WarningKind::LowOnPaper => fl!("low-on-paper"),
        WarningKind::OutOfToner => fl!("out-of-toner"),
        WarningKind::LowToner => fl!("low-toner"),
        WarningKind::CartridgeMissing => fl!("cartridge-missing"),
        WarningKind::CoverOpen => fl!("cover-open"),
        WarningKind::Offline => fl!("offline"),
        WarningKind::Connecting => fl!("connecting"),
        WarningKind::MissingFilter => fl!("missing-filter"),
        WarningKind::Paused => fl!("paused"),
        WarningKind::PrinterError => fl!("printer-error"),
        WarningKind::Stopped => fl!("stopped"),
        WarningKind::NotAccepting => fl!("not-accepting"),
    }
}

fn queue_error_text(error: QueueError) -> String {
    match error {
        QueueError::NoClient => fl!("no-cups-client"),
        QueueError::ListFailed => fl!("list-failed"),
    }
}

fn count_arg(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

fn cups_subscription() -> Subscription<Message> {
    Subscription::run(cups_stream)
}

fn cups_stream() -> impl futures::stream::Stream<Item = Message> + Send + 'static {
    cosmic::iced::stream::channel(64, async |mut output| {
        loop {
            let mut notifier = match cups::Notifier::connect().await {
                Ok(notifier) => notifier,
                Err(error) => {
                    eprintln!("cosmic-printers: {error}");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };
            while let Some(signal) = notifier.recv().await {
                if output.send(Message::Cups(signal)).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    })
}
