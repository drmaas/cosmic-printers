// SPDX-License-Identifier: GPL-3.0-only

//! Pure printer status, icon choice, and queue ordering.

use crate::config::ShowIcon;

pub const PRINTER_STATE_IDLE: u32 = 3;
pub const PRINTER_STATE_PROCESSING: u32 = 4;
pub const PRINTER_STATE_STOPPED: u32 = 5;
pub const JOB_STATE_PENDING: u32 = 3;
pub const JOB_STATE_PROCESSING: u32 = 5;
pub const JOB_STATE_CANCELED: u32 = 7;

pub const ICON_IDLE: &str = "printer-symbolic";
pub const ICON_PRINTING: &str = "printer-printing-symbolic";
pub const ICON_WARNING: &str = "printer-warning-symbolic";
pub const ICON_ERROR: &str = "printer-error-symbolic";
pub const ICON_DOCUMENTS: &str = "emblem-documents-symbolic";
pub const ICON_RUNNING: &str = "media-playback-start-symbolic";

const REASONS: &[(&str, WarningKind)] = &[
    ("media-empty", WarningKind::OutOfPaper),
    ("media-needed", WarningKind::OutOfPaper),
    ("media-jam", WarningKind::PaperJam),
    ("media-low", WarningKind::LowOnPaper),
    ("toner-empty", WarningKind::OutOfToner),
    ("toner-low", WarningKind::LowToner),
    ("marker-supply-empty", WarningKind::OutOfToner),
    ("marker-supply-low", WarningKind::LowToner),
    ("marker-supply-missing", WarningKind::CartridgeMissing),
    ("cover-open", WarningKind::CoverOpen),
    ("door-open", WarningKind::CoverOpen),
    ("offline", WarningKind::Offline),
    ("connecting-to-device", WarningKind::Connecting),
    ("cups-missing-filter", WarningKind::MissingFilter),
    ("paused", WarningKind::Paused),
    ("other", WarningKind::PrinterError),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarningKind {
    OutOfPaper,
    PaperJam,
    LowOnPaper,
    OutOfToner,
    LowToner,
    CartridgeMissing,
    CoverOpen,
    Offline,
    Connecting,
    MissingFilter,
    Paused,
    PrinterError,
    Stopped,
    NotAccepting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Warning {
    pub kind: WarningKind,
    pub error: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusInput {
    pub display_name: String,
    pub state: u32,
    pub state_reasons: String,
    pub accepting: bool,
    pub job_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelStatus {
    pub icon: &'static str,
    pub tooltip: Tooltip,
    pub job_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tooltip {
    Printers,
    Jobs(usize),
    Warning { printer: String, kind: WarningKind },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PanelWarning {
    printer: String,
    kind: WarningKind,
    error: bool,
}

#[must_use]
pub fn display_name(name: &str) -> String {
    name.replace('_', " ")
}

#[must_use]
pub fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut truncated: String = value.chars().take(max).collect();
    truncated.push_str("...");
    truncated
}

#[must_use]
pub fn printer_warning(state_reasons: &str) -> Option<Warning> {
    if state_reasons.is_empty() || state_reasons == "none" {
        return None;
    }

    for reason in state_reasons.split(',') {
        let trimmed = reason.trim();
        if trimmed.is_empty() || trimmed == "none" {
            continue;
        }
        let (base, error) = split_reason(trimmed);
        if let Some(kind) = kind_for(base) {
            return Some(Warning { kind, error });
        }
    }
    None
}

#[must_use]
pub fn row_notice(state_reasons: &str, accepting: bool) -> Option<WarningKind> {
    if let Some(warning) = printer_warning(state_reasons) {
        return Some(warning.kind);
    }
    if !accepting {
        return Some(WarningKind::NotAccepting);
    }
    None
}

#[must_use]
pub fn printer_icon(state: u32, state_reasons: &str) -> &'static str {
    if state == PRINTER_STATE_STOPPED {
        return ICON_ERROR;
    }
    match printer_warning(state_reasons) {
        Some(warning) if warning.error => ICON_ERROR,
        Some(_) => ICON_WARNING,
        None => ICON_DOCUMENTS,
    }
}

#[must_use]
pub fn panel_status(printers: &[StatusInput]) -> PanelStatus {
    let job_count = printers.iter().map(|printer| printer.job_count).sum();
    let mut worst: Option<PanelWarning> = None;
    let mut stopped = false;

    for printer in printers {
        let warning = printer_warning(&printer.state_reasons);
        if printer.state == PRINTER_STATE_STOPPED {
            if !stopped {
                let (kind, error) = match warning {
                    Some(warning) => (warning.kind, true),
                    None => (WarningKind::Stopped, true),
                };
                worst = Some(PanelWarning {
                    printer: printer.display_name.clone(),
                    kind,
                    error,
                });
                stopped = true;
            }
        } else if let Some(warning) = warning
            && !stopped
        {
            let replace = match &worst {
                None => true,
                Some(_) if warning.error => true,
                Some(_) => false,
            };
            if replace {
                worst = Some(PanelWarning {
                    printer: printer.display_name.clone(),
                    kind: warning.kind,
                    error: warning.error,
                });
            }
        }
    }

    let icon = if stopped || worst.as_ref().is_some_and(|item| item.error) {
        ICON_ERROR
    } else if worst.is_some() {
        ICON_WARNING
    } else if job_count > 0 {
        ICON_PRINTING
    } else {
        ICON_IDLE
    };

    let tooltip = if let Some(worst) = worst {
        Tooltip::Warning {
            printer: worst.printer,
            kind: worst.kind,
        }
    } else if job_count > 0 {
        Tooltip::Jobs(job_count)
    } else {
        Tooltip::Printers
    };

    PanelStatus {
        icon,
        tooltip,
        job_count,
    }
}

#[must_use]
pub fn icon_visible(show: ShowIcon, printer_count: usize, job_count: usize) -> bool {
    match show {
        ShowIcon::Always => true,
        ShowIcon::Printers => printer_count > 0,
        ShowIcon::Jobs => job_count > 0,
    }
}

/// Queued jobs can move ahead of other queued jobs. A job already printing stays in front.
#[must_use]
pub fn can_send_to_front(index: usize, first_is_processing: bool) -> bool {
    index > 0 && !(index == 1 && first_is_processing)
}

#[must_use]
pub fn front_insert_index(first_is_processing: bool) -> usize {
    usize::from(first_is_processing)
}

fn split_reason(reason: &str) -> (&str, bool) {
    match reason.rsplit_once('-') {
        Some((base, "error")) => (base, true),
        Some((base, "warning" | "report")) => (base, false),
        _ => (reason, false),
    }
}

fn kind_for(base: &str) -> Option<WarningKind> {
    REASONS
        .iter()
        .find_map(|(name, kind)| (*name == base).then_some(*kind))
}

#[cfg(test)]
mod tests {
    use super::{
        ICON_ERROR, ICON_IDLE, ICON_PRINTING, ICON_WARNING, PRINTER_STATE_IDLE,
        PRINTER_STATE_PROCESSING, PRINTER_STATE_STOPPED, PanelStatus, ShowIcon, StatusInput,
        Tooltip, WarningKind, can_send_to_front, display_name, front_insert_index, icon_visible,
        panel_status, printer_icon, printer_warning, truncate_chars,
    };

    fn printer(name: &str, state: u32, reasons: &str, jobs: usize) -> StatusInput {
        StatusInput {
            display_name: name.to_string(),
            state,
            state_reasons: reasons.to_string(),
            accepting: true,
            job_count: jobs,
        }
    }

    #[test]
    fn maps_reason_suffixes() {
        let warning = printer_warning("none, media-empty-error").unwrap();
        assert_eq!(warning.kind, WarningKind::OutOfPaper);
        assert!(warning.error);

        let warning = printer_warning("toner-low-warning").unwrap();
        assert_eq!(warning.kind, WarningKind::LowToner);
        assert!(!warning.error);

        assert_eq!(printer_warning("none"), None);
        assert_eq!(printer_warning("not-a-real-reason-warning"), None);
    }

    #[test]
    fn display_name_replaces_underscores() {
        assert_eq!(display_name("Office_Laser"), "Office Laser");
    }

    #[test]
    fn truncates_on_character_boundary() {
        assert_eq!(truncate_chars("abcdef", 3), "abc...");
        assert_eq!(truncate_chars("ab", 3), "ab");
    }

    #[test]
    fn panel_prefers_stopped_then_error_then_jobs() {
        let idle = panel_status(&[printer("Office", PRINTER_STATE_IDLE, "none", 0)]);
        assert_eq!(
            idle,
            PanelStatus {
                icon: ICON_IDLE,
                tooltip: Tooltip::Printers,
                job_count: 0,
            }
        );

        let printing = panel_status(&[printer("Office", PRINTER_STATE_PROCESSING, "none", 2)]);
        assert_eq!(printing.icon, ICON_PRINTING);
        assert_eq!(printing.tooltip, Tooltip::Jobs(2));

        let low = panel_status(&[printer(
            "Office",
            PRINTER_STATE_IDLE,
            "media-low-warning",
            1,
        )]);
        assert_eq!(low.icon, ICON_WARNING);
        assert_eq!(
            low.tooltip,
            Tooltip::Warning {
                printer: "Office".to_string(),
                kind: WarningKind::LowOnPaper,
            }
        );

        let stopped = panel_status(&[
            printer("Office", PRINTER_STATE_IDLE, "toner-empty-error", 1),
            printer("Photo", PRINTER_STATE_STOPPED, "none", 0),
        ]);
        assert_eq!(stopped.icon, ICON_ERROR);
        assert_eq!(
            stopped.tooltip,
            Tooltip::Warning {
                printer: "Photo".to_string(),
                kind: WarningKind::Stopped,
            }
        );
        assert_eq!(printer_icon(PRINTER_STATE_STOPPED, "none"), ICON_ERROR);
    }

    #[test]
    fn later_error_replaces_an_earlier_warning() {
        let status = panel_status(&[
            printer("Office", PRINTER_STATE_IDLE, "media-low-warning", 0),
            printer("Photo", PRINTER_STATE_IDLE, "offline-error", 0),
        ]);
        assert_eq!(
            status.tooltip,
            Tooltip::Warning {
                printer: "Photo".to_string(),
                kind: WarningKind::Offline,
            }
        );
    }

    #[test]
    fn visibility_follows_show_icon() {
        assert!(icon_visible(ShowIcon::Always, 0, 0));
        assert!(!icon_visible(ShowIcon::Printers, 0, 0));
        assert!(icon_visible(ShowIcon::Printers, 1, 0));
        assert!(!icon_visible(ShowIcon::Jobs, 2, 0));
        assert!(icon_visible(ShowIcon::Jobs, 0, 1));
    }

    #[test]
    fn send_to_front_index_skips_the_active_job() {
        assert!(!can_send_to_front(0, false));
        assert!(can_send_to_front(1, false));
        assert!(!can_send_to_front(1, true));
        assert!(can_send_to_front(2, true));
        assert_eq!(front_insert_index(false), 0);
        assert_eq!(front_insert_index(true), 1);
    }
}
