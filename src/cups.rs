// SPDX-License-Identifier: GPL-3.0-only

//! CUPS queue model, `lpstat` bootstrap, and the cupsd notifier.

use std::collections::HashSet;
use std::process::{Command, Stdio};

use futures::stream::StreamExt;
use indexmap::IndexMap;

use crate::status::{
    self, JOB_STATE_CANCELED, JOB_STATE_PENDING, JOB_STATE_PROCESSING, display_name,
    front_insert_index,
};

const NOTIFIER_INTERFACE: &str = "org.cups.cupsd.Notifier";
const NOTIFIER_PATH: &str = "/org/cups/cupsd/Notifier";
const PRINTING_CONFIG_BUS: &str = "org.fedoraproject.Config.Printing";
const PRINTING_CONFIG_PATH: &str = "/org/fedoraproject/Config/Printing";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueError {
    NoClient,
    ListFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: u32,
    pub name: String,
    pub state: u32,
    pub created_display: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printer {
    pub name: String,
    pub display_name: String,
    pub uri: String,
    pub state: u32,
    pub state_reasons: String,
    pub accepting: bool,
    pub jobs: IndexMap<u32, Job>,
}

impl Printer {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            display_name: display_name(name),
            uri: String::new(),
            state: 0,
            state_reasons: "none".to_string(),
            accepting: true,
            jobs: IndexMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub printers: IndexMap<String, Printer>,
    pub error: Option<QueueError>,
    /// Queue names from `lpstat -p`. While non-empty, discovered-only destinations are hidden.
    installed_queues: HashSet<String>,
    /// Filtered `lpstat -e` names mapped onto an installed queue with the same device.
    destination_aliases: IndexMap<String, String>,
}

impl Queue {
    pub fn empty() -> Self {
        Self {
            printers: IndexMap::new(),
            error: None,
            installed_queues: HashSet::new(),
            destination_aliases: IndexMap::new(),
        }
    }

    fn with_error(error: QueueError) -> Self {
        Self {
            printers: IndexMap::new(),
            error: Some(error),
            installed_queues: HashSet::new(),
            destination_aliases: IndexMap::new(),
        }
    }

    pub fn handle_signal(&mut self, signal: CupsSignal) -> ApplyResult {
        match signal {
            CupsSignal::ServerReset => {
                self.printers.clear();
                self.installed_queues.clear();
                self.destination_aliases.clear();
                self.error = None;
                ApplyResult::Reset
            }
            CupsSignal::ServerStopped => {
                let changed = !self.printers.is_empty();
                self.printers.clear();
                if changed {
                    ApplyResult::Changed
                } else {
                    ApplyResult::Unchanged
                }
            }
            CupsSignal::PrinterDeleted { name } => {
                if name.is_empty() {
                    return ApplyResult::Unchanged;
                }
                self.note_signal();
                if self.printers.shift_remove(&name).is_some() {
                    ApplyResult::Changed
                } else {
                    ApplyResult::Unchanged
                }
            }
            CupsSignal::PrinterUpsert {
                name,
                uri,
                state,
                reasons,
                accepting,
                newly_added,
            } => {
                if name.is_empty() {
                    return ApplyResult::Unchanged;
                }
                self.note_signal();
                if newly_added {
                    self.refresh_installed_queues();
                }
                let Some(resolved) = self.resolve_printer_name(&name, &uri) else {
                    return ApplyResult::Unchanged;
                };
                if self.update_printer(&resolved, &uri, state, &reasons, accepting) {
                    ApplyResult::Changed
                } else {
                    ApplyResult::Unchanged
                }
            }
            CupsSignal::JobUpsert {
                printer,
                printer_uri,
                printer_state,
                printer_reasons,
                accepting,
                job_id,
                job_state,
                job_name,
            } => {
                if printer.is_empty() {
                    return ApplyResult::Unchanged;
                }
                self.note_signal();
                let Some(resolved) = self.resolve_printer_name(&printer, &printer_uri) else {
                    return ApplyResult::Unchanged;
                };
                let mut changed = self.update_printer(
                    &resolved,
                    &printer_uri,
                    printer_state,
                    &printer_reasons,
                    accepting,
                );
                changed |= self.update_job(&resolved, job_id, job_state, &job_name);
                if changed {
                    ApplyResult::Changed
                } else {
                    ApplyResult::Unchanged
                }
            }
            CupsSignal::JobRemoved {
                printer,
                printer_uri,
                printer_state,
                printer_reasons,
                accepting,
                job_id,
            } => {
                if printer.is_empty() {
                    return ApplyResult::Unchanged;
                }
                self.note_signal();
                let Some(resolved) = self.resolve_printer_name(&printer, &printer_uri) else {
                    return ApplyResult::Unchanged;
                };
                let mut changed = self.update_printer(
                    &resolved,
                    &printer_uri,
                    printer_state,
                    &printer_reasons,
                    accepting,
                );
                if self
                    .printers
                    .get_mut(&resolved)
                    .is_some_and(|item| item.jobs.shift_remove(&job_id).is_some())
                {
                    changed = true;
                }
                if changed {
                    ApplyResult::Changed
                } else {
                    ApplyResult::Unchanged
                }
            }
        }
    }

    fn note_signal(&mut self) {
        self.error = None;
    }

    fn refresh_installed_queues(&mut self) {
        let output = match run_command("lpstat", &["-l", "-p"]) {
            CommandResult::Ok(text) => text,
            CommandResult::Failed(_) | CommandResult::Missing => return,
        };
        self.installed_queues = installed_printer_names(&output).into_iter().collect();
    }

    /// Maps a notifier or lpstat job printer name to the queue row that should hold it.
    fn resolve_printer_name(&self, name: &str, uri: &str) -> Option<String> {
        if self.installed_queues.is_empty() {
            return Some(name.to_string());
        }
        if self.installed_queues.contains(name) {
            return Some(name.to_string());
        }
        if let Some(installed) = self.destination_aliases.get(name) {
            return Some(installed.clone());
        }
        self.match_installed_by_uri(uri)
    }

    fn match_installed_by_uri(&self, uri: &str) -> Option<String> {
        if uri.is_empty() {
            return None;
        }
        self.installed_queues.iter().find_map(|installed| {
            let printer = self.printers.get(installed)?;
            if printer.uri.is_empty() {
                return None;
            }
            uris_share_device(uri, &printer.uri).then(|| installed.clone())
        })
    }

    fn resolve_job_printer(
        &self,
        name: &str,
        signal_uri: &str,
        bootstrap_uris: &IndexMap<String, String>,
    ) -> Option<String> {
        if self.installed_queues.is_empty() {
            return Some(name.to_string());
        }
        if self.installed_queues.contains(name) {
            return Some(name.to_string());
        }
        if let Some(installed) = self.destination_aliases.get(name) {
            return Some(installed.clone());
        }
        self.match_installed_by_uri(signal_uri).or_else(|| {
            bootstrap_uris
                .get(name)
                .and_then(|uri| self.match_installed_by_uri(uri))
        })
    }

    fn update_printer(
        &mut self,
        name: &str,
        uri: &str,
        state: u32,
        reasons: &str,
        accepting: bool,
    ) -> bool {
        if !self.installed_queues.is_empty() && !self.installed_queues.contains(name) {
            return false;
        }
        let printer = self
            .printers
            .entry(name.to_string())
            .or_insert_with(|| Printer::new(name));
        let mut changed = false;
        if !uri.is_empty() && printer.uri != uri {
            printer.uri = uri.to_string();
            changed = true;
        }
        if printer.state != state {
            printer.state = state;
            changed = true;
        }
        if printer.state_reasons != reasons {
            printer.state_reasons = reasons.to_string();
            changed = true;
        }
        if printer.accepting != accepting {
            printer.accepting = accepting;
            changed = true;
        }
        changed
    }

    fn update_job(
        &mut self,
        printer_name: &str,
        job_id: u32,
        job_state: u32,
        job_name: &str,
    ) -> bool {
        let Some(printer) = self.printers.get_mut(printer_name) else {
            return false;
        };
        if job_state >= JOB_STATE_CANCELED {
            return printer.jobs.shift_remove(&job_id).is_some();
        }

        let created_display = printer
            .jobs
            .get(&job_id)
            .map(|job| job.created_display.clone())
            .unwrap_or_else(local_now_display);
        let name = if job_name.is_empty() {
            printer
                .jobs
                .get(&job_id)
                .map(|job| job.name.clone())
                .unwrap_or_default()
        } else {
            job_name.to_string()
        };
        let existing = printer.jobs.get(&job_id);
        if existing.is_some_and(|job| job.state == job_state && job.name == name) {
            return false;
        }
        printer.jobs.insert(
            job_id,
            Job {
                id: job_id,
                name,
                state: job_state,
                created_display,
            },
        );
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyResult {
    Unchanged,
    Changed,
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CupsSignal {
    ServerReset,
    ServerStopped,
    PrinterUpsert {
        name: String,
        uri: String,
        state: u32,
        reasons: String,
        accepting: bool,
        newly_added: bool,
    },
    PrinterDeleted {
        name: String,
    },
    JobUpsert {
        printer: String,
        printer_uri: String,
        printer_state: u32,
        printer_reasons: String,
        accepting: bool,
        job_id: u32,
        job_state: u32,
        job_name: String,
    },
    JobRemoved {
        printer: String,
        printer_uri: String,
        printer_state: u32,
        printer_reasons: String,
        accepting: bool,
        job_id: u32,
    },
}

pub struct Notifier {
    stream: zbus::MessageStream,
}

impl Notifier {
    pub async fn connect() -> Result<Self, String> {
        let connection = zbus::Connection::system()
            .await
            .map_err(|error| error.to_string())?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(NOTIFIER_INTERFACE)
            .path(NOTIFIER_PATH)
            .build()
            .map_err(|error| error.to_string())?;
        let stream = zbus::MessageStream::for_match_rule(rule, &connection, None)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self { stream })
    }

    pub async fn recv(&mut self) -> Option<CupsSignal> {
        while let Some(next) = self.stream.next().await {
            match next {
                Ok(message) => {
                    if let Some(signal) = decode(&message) {
                        return Some(signal);
                    }
                }
                Err(error) => {
                    eprintln!("cosmic-printers: CUPS notifier: {error}");
                    return None;
                }
            }
        }
        None
    }
}

pub async fn bootstrap() -> Queue {
    tokio::task::spawn_blocking(read_queue)
        .await
        .unwrap_or_else(|_| Queue::with_error(QueueError::ListFailed))
}

pub fn send_job_to_front(jobs: &mut IndexMap<u32, Job>, job_id: u32) {
    let Some(from) = jobs.get_index_of(&job_id) else {
        return;
    };
    let first_processing = jobs
        .first()
        .is_some_and(|(_, job)| job.state == JOB_STATE_PROCESSING);
    let to = front_insert_index(first_processing);
    if from > to {
        jobs.move_index(from, to);
    }
}

pub fn cancel_job(printer: &str, job_id: u32) -> Result<(), String> {
    let id = format!("{printer}-{job_id}");
    match run_command("cancel", &[&id]) {
        CommandResult::Ok(_) => Ok(()),
        CommandResult::Missing => Err("cancel was not found".to_string()),
        CommandResult::Failed(error) => Err(error),
    }
}

pub fn prioritize_job(printer: &str, job_id: u32) -> Result<(), String> {
    let id = format!("{printer}-{job_id}");
    match run_command("lp", &["-i", &id, "-q", "100"]) {
        CommandResult::Ok(_) => Ok(()),
        CommandResult::Missing => Err("lp was not found".to_string()),
        CommandResult::Failed(error) => Err(error),
    }
}

pub fn open_config_tool(printer: Option<&str>) -> Result<(), String> {
    let mut args = Vec::new();
    if let Some(name) = printer {
        args.push("--show-jobs".to_string());
        args.push(name.to_string());
    }
    spawn_detached("system-config-printer", &args)
}

pub async fn open_properties(printer: &str) -> Result<(), String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|error| error.to_string())?;
    connection
        .call_method(
            Some(PRINTING_CONFIG_BUS),
            PRINTING_CONFIG_PATH,
            Some(PRINTING_CONFIG_BUS),
            "PrinterPropertiesDialog",
            &(0u32, printer),
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tools {
    pub config_tool: bool,
    pub properties: bool,
}

pub async fn detect_tools() -> Tools {
    Tools {
        config_tool: program_in_path("system-config-printer"),
        properties: properties_available().await,
    }
}

pub fn program_in_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

fn read_queue() -> Queue {
    let destinations = run_command("lpstat", &["-e"]);
    if matches!(destinations, CommandResult::Missing) {
        return Queue::with_error(QueueError::NoClient);
    }
    let printers = run_command("lpstat", &["-l", "-p"]);
    let devices = run_command("lpstat", &["-v"]);
    let jobs = run_command("lpstat", &["-W", "not-completed", "-o"]);
    for result in [&destinations, &printers, &devices, &jobs] {
        if let CommandResult::Failed(error) = result {
            eprintln!("cosmic-printers: {error}");
        }
    }

    let mut queue = queue_from_outputs(
        command_text(&destinations),
        command_text(&printers),
        command_text(&jobs),
        command_text(&devices),
    );
    if !matches!(destinations, CommandResult::Ok(_))
        && !matches!(printers, CommandResult::Ok(_))
        && queue.printers.is_empty()
    {
        queue.error = Some(QueueError::ListFailed);
    }
    queue
}

fn command_text(result: &CommandResult) -> &str {
    match result {
        CommandResult::Ok(text) => text,
        CommandResult::Failed(_) | CommandResult::Missing => "",
    }
}

/// Installed queues from `lpstat -p` are the printer list. CUPS also advertises
/// temporary DNS-SD destinations on `lpstat -e`; those are hidden unless they
/// appear as installed queues, so the same device is never listed twice.
fn queue_from_outputs(destinations: &str, printers: &str, jobs: &str, devices: &str) -> Queue {
    let mut queue = Queue::empty();
    let uris = parse_device_uri_map(devices);
    queue.installed_queues = installed_printer_names(printers).into_iter().collect();
    if !queue.installed_queues.is_empty() {
        for dest in parse_destination_list(destinations) {
            if queue.installed_queues.contains(&dest) {
                continue;
            }
            let Some(installed) = queue
                .installed_queues
                .iter()
                .find(|installed| destinations_share_device(installed, &dest, &uris))
            else {
                continue;
            };
            queue.destination_aliases.insert(dest, (*installed).clone());
        }
    }
    for name in queue_printer_names(destinations, printers, &uris) {
        let printer = queue
            .printers
            .entry(name.clone())
            .or_insert_with(|| Printer::new(&name));
        if let Some(uri) = uris.get(&name)
            && printer.uri.is_empty()
        {
            printer.uri = uri.clone();
        }
    }

    let mut current: Option<String> = None;
    let mut printing_jobs = Vec::new();
    for line in printers.lines() {
        if let Some(parsed) = parse_printer_line(line) {
            let printer = queue
                .printers
                .entry(parsed.name.clone())
                .or_insert_with(|| Printer::new(&parsed.name));
            printer.state = parsed.state;
            printer.accepting = parsed.accepting;
            if let Some(job_id) = parsed.printing_job {
                printing_jobs.push((parsed.name.clone(), job_id));
            }
            current = Some(parsed.name);
        } else if let Some(reasons) = parse_alerts_line(line)
            && let Some(name) = current.clone()
            && let Some(printer) = queue.printers.get_mut(&name)
        {
            printer.state_reasons = reasons;
        }
    }

    for job in parse_job_lines(jobs) {
        let Some(printer_name) = queue.resolve_job_printer(&job.printer, "", &uris) else {
            continue;
        };
        let printer = queue
            .printers
            .entry(printer_name.clone())
            .or_insert_with(|| Printer::new(&printer_name));
        printer.jobs.insert(
            job.id,
            Job {
                id: job.id,
                name: String::new(),
                state: JOB_STATE_PENDING,
                created_display: job.created_display,
            },
        );
    }
    for (name, job_id) in printing_jobs {
        if let Some(job) = queue
            .printers
            .get_mut(&name)
            .and_then(|printer| printer.jobs.get_mut(&job_id))
        {
            job.state = JOB_STATE_PROCESSING;
        }
    }
    queue
}

struct ParsedPrinterLine {
    name: String,
    state: u32,
    accepting: bool,
    printing_job: Option<u32>,
}

struct ParsedJobLine {
    printer: String,
    id: u32,
    created_display: String,
}

fn parse_destination_list(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn installed_printer_names(printers: &str) -> Vec<String> {
    printers
        .lines()
        .filter_map(|line| parse_printer_line(line).map(|parsed| parsed.name))
        .collect()
}

fn parse_device_uri_map(output: &str) -> IndexMap<String, String> {
    let mut map = IndexMap::new();
    for line in output.lines() {
        let Some(rest) = line.trim().strip_prefix("device for ") else {
            continue;
        };
        let Some((name, uri)) = rest.rsplit_once(": ") else {
            continue;
        };
        if !name.is_empty() && !uri.is_empty() {
            map.insert(name.to_string(), uri.trim().to_string());
        }
    }
    map
}

fn uris_share_device(left: &str, right: &str) -> bool {
    let left_ids = device_identities(left);
    let right_ids = device_identities(right);
    left_ids
        .iter()
        .any(|id| right_ids.iter().any(|other| id == other))
}

fn device_identities(uri: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let lower = uri.to_ascii_lowercase();
    if let Some(uuid) = query_param_value(&lower, "uuid") {
        ids.push(uuid);
    }
    if let Some(host) = uri_authority_host(uri) {
        ids.push(host.clone());
        if let Some(stem) = host.strip_suffix(".local") {
            ids.push(stem.to_string());
        }
    }
    ids
}

fn query_param_value(uri: &str, key: &str) -> Option<String> {
    let query = uri.split('?').nth(1)?;
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name.eq_ignore_ascii_case(key)).then(|| value.to_ascii_lowercase())
    })
}

fn uri_authority_host(uri: &str) -> Option<String> {
    let after_scheme = uri.split_once("://")?.1;
    let authority = after_scheme.split(['/', '?', '#']).next()?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host
        .rsplit_once(']')
        .map_or(host, |(left, _)| left.strip_prefix('[').unwrap_or(left));
    let host = host
        .rsplit_once(':')
        .map_or(host, |(name, port)| {
            if name.contains('.') || name.contains(':') || port.chars().all(|c| c.is_ascii_digit())
            {
                name
            } else {
                authority
            }
        })
        .to_ascii_lowercase();
    if host.is_empty() { None } else { Some(host) }
}

fn destinations_share_device(left: &str, right: &str, uris: &IndexMap<String, String>) -> bool {
    let Some(left_ids) = uris.get(left).map(|uri| device_identities(uri)) else {
        return false;
    };
    let Some(right_ids) = uris.get(right).map(|uri| device_identities(uri)) else {
        return false;
    };
    left_ids
        .iter()
        .any(|id| right_ids.iter().any(|other| id == other))
}

fn queue_printer_names(
    destinations: &str,
    printers: &str,
    uris: &IndexMap<String, String>,
) -> Vec<String> {
    let installed = installed_printer_names(printers);
    if !installed.is_empty() {
        return installed;
    }

    // `lpstat -p` failed or returned no queues; fall back to `lpstat -e` while
    // collapsing destinations that resolve to the same device URI identity.
    let mut list: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for name in parse_destination_list(destinations) {
        if list
            .iter()
            .any(|kept| destinations_share_device(kept, &name, uris))
        {
            continue;
        }
        if let Some(uri) = uris.get(&name) {
            let overlaps = device_identities(uri).iter().any(|id| seen.contains(id));
            if overlaps {
                continue;
            }
            for id in device_identities(uri) {
                seen.insert(id);
            }
        }
        list.push(name);
    }
    list
}

fn parse_printer_line(line: &str) -> Option<ParsedPrinterLine> {
    let rest = line.strip_prefix("printer ")?;
    const MARKERS: &[(&str, u32, bool)] = &[
        (" is holding new jobs.", status::PRINTER_STATE_IDLE, false),
        (" is idle.", status::PRINTER_STATE_IDLE, true),
        (" now printing ", status::PRINTER_STATE_PROCESSING, true),
        (" disabled since ", status::PRINTER_STATE_STOPPED, true),
    ];
    for (marker, state, accepting) in MARKERS {
        if let Some(index) = rest.find(marker) {
            let name = &rest[..index];
            if name.is_empty() {
                return None;
            }
            let printing_job = (*marker == " now printing ")
                .then(|| printing_job_id(&rest[index + marker.len()..]))
                .flatten();
            return Some(ParsedPrinterLine {
                name: name.to_string(),
                state: *state,
                accepting: *accepting,
                printing_job,
            });
        }
    }
    None
}

fn printing_job_id(rest: &str) -> Option<u32> {
    let token = rest.split_whitespace().next()?.trim_end_matches('.');
    split_job_token(token).map(|(_, id)| id)
}

fn parse_alerts_line(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("Alerts:")?;
    let reasons: Vec<&str> = rest
        .split_whitespace()
        .filter(|token| *token != "none")
        .collect();
    if reasons.is_empty() {
        None
    } else {
        Some(reasons.join(","))
    }
}

fn parse_job_lines(output: &str) -> Vec<ParsedJobLine> {
    output.lines().filter_map(parse_job_line).collect()
}

fn parse_job_line(line: &str) -> Option<ParsedJobLine> {
    let line = line.trim();
    if line.is_empty() || line.starts_with("printer ") {
        return None;
    }
    let mut parts = line.split_whitespace();
    let token = parts.next()?;
    let _user = parts.next()?;
    let _size = parts.next()?;
    let created_display = parts.collect::<Vec<_>>().join(" ");
    if created_display.is_empty() {
        return None;
    }
    let (printer, id) = split_job_token(token)?;
    Some(ParsedJobLine {
        printer: printer.to_string(),
        id,
        created_display,
    })
}

fn split_job_token(token: &str) -> Option<(&str, u32)> {
    let (name, id) = token.rsplit_once('-')?;
    if name.is_empty() {
        return None;
    }
    let id = id.parse().ok()?;
    Some((name, id))
}

fn local_now_display() -> String {
    jiff::Zoned::now()
        .strftime("%a %b %d %H:%M:%S %Y")
        .to_string()
}

enum CommandResult {
    Ok(String),
    Failed(String),
    Missing,
}

fn run_command(program: &str, args: &[&str]) -> CommandResult {
    match Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            if output.status.success() {
                CommandResult::Ok(stdout)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                CommandResult::Failed(format!(
                    "{program} exited with {}: {}",
                    output.status,
                    stderr.trim()
                ))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CommandResult::Missing,
        Err(error) => CommandResult::Failed(error.to_string()),
    }
}

fn spawn_detached(program: &str, args: &[String]) -> Result<(), String> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

async fn properties_available() -> bool {
    let Ok(connection) = zbus::Connection::session().await else {
        return false;
    };
    let Ok(dbus) = zbus::fdo::DBusProxy::new(&connection).await else {
        return false;
    };
    let mut names = Vec::new();
    if let Ok(activatable) = dbus.list_activatable_names().await {
        names.extend(activatable);
    }
    if let Ok(owned) = dbus.list_names().await {
        names.extend(owned);
    }
    names
        .iter()
        .any(|name| name.as_str() == PRINTING_CONFIG_BUS)
}

fn decode(message: &zbus::Message) -> Option<CupsSignal> {
    let member = message.header().member()?.to_string();
    match member.as_str() {
        "ServerStarted" | "ServerRestarted" => Some(CupsSignal::ServerReset),
        "ServerStopped" => Some(CupsSignal::ServerStopped),
        "ServerAudit" => None,
        "PrinterAdded" => decode_printer(message, true),
        "PrinterModified"
        | "PrinterRestarted"
        | "PrinterStopped"
        | "PrinterShutdown"
        | "PrinterStateChanged"
        | "PrinterFinishingsChanged"
        | "PrinterMediaChanged" => decode_printer(message, false),
        "PrinterDeleted" => decode_printer_deleted(message),
        "JobCreated" | "JobState" | "JobConfigChanged" | "JobProgress" => decode_job(message),
        "JobCompleted" | "JobStopped" => decode_job_removed(message),
        _ => None,
    }
}

fn decode_printer(message: &zbus::Message, newly_added: bool) -> Option<CupsSignal> {
    let (_text, uri, name, state, reasons, accepting): (String, String, String, u32, String, bool) =
        message.body().deserialize().ok()?;
    if name.is_empty() {
        return None;
    }
    Some(CupsSignal::PrinterUpsert {
        name,
        uri,
        state,
        reasons,
        accepting,
        newly_added,
    })
}

fn decode_printer_deleted(message: &zbus::Message) -> Option<CupsSignal> {
    let (_text, _uri, name, _state, _reasons, _accepting): (
        String,
        String,
        String,
        u32,
        String,
        bool,
    ) = message.body().deserialize().ok()?;
    Some(CupsSignal::PrinterDeleted { name })
}

fn decode_job(message: &zbus::Message) -> Option<CupsSignal> {
    let (
        _text,
        printer_uri,
        printer,
        printer_state,
        printer_reasons,
        accepting,
        job_id,
        job_state,
        _job_reasons,
        job_name,
        _impressions,
    ): (
        String,
        String,
        String,
        u32,
        String,
        bool,
        u32,
        u32,
        String,
        String,
        u32,
    ) = message.body().deserialize().ok()?;
    if printer.is_empty() {
        return None;
    }
    Some(CupsSignal::JobUpsert {
        printer,
        printer_uri,
        printer_state,
        printer_reasons,
        accepting,
        job_id,
        job_state,
        job_name,
    })
}

fn decode_job_removed(message: &zbus::Message) -> Option<CupsSignal> {
    let (
        _text,
        printer_uri,
        printer,
        printer_state,
        printer_reasons,
        accepting,
        job_id,
        _job_state,
        _job_reasons,
        _job_name,
        _impressions,
    ): (
        String,
        String,
        String,
        u32,
        String,
        bool,
        u32,
        u32,
        String,
        String,
        u32,
    ) = message.body().deserialize().ok()?;
    // JobCompleted still carries the full job body. Fall back to the shorter form if that fails.
    if printer.is_empty() {
        return None;
    }
    Some(CupsSignal::JobRemoved {
        printer,
        printer_uri,
        printer_state,
        printer_reasons,
        accepting,
        job_id,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ApplyResult, CupsSignal, Job, Queue, device_identities, parse_alerts_line,
        parse_destination_list, parse_device_uri_map, parse_job_line, parse_printer_line,
        queue_from_outputs, queue_printer_names, send_job_to_front,
    };
    use crate::status::{
        JOB_STATE_CANCELED, JOB_STATE_PROCESSING, PRINTER_STATE_IDLE, PRINTER_STATE_STOPPED,
    };
    use indexmap::IndexMap;

    fn jobs_with_states(states: &[u32]) -> IndexMap<u32, Job> {
        states
            .iter()
            .enumerate()
            .map(|(index, state)| {
                let id = u32::try_from(index).unwrap() + 1;
                (
                    id,
                    Job {
                        id,
                        name: format!("job {id}"),
                        state: *state,
                        created_display: "now".to_string(),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn parses_lpstat_sections() {
        assert_eq!(
            parse_destination_list("Office_Laser\nPhoto\n"),
            vec!["Office_Laser".to_string(), "Photo".to_string()]
        );
        let idle = parse_printer_line(
            "printer Office_Laser is idle.  enabled since Tue Nov 10 10:00:00 2026",
        )
        .unwrap();
        assert_eq!(idle.name, "Office_Laser");
        assert_eq!(idle.state, PRINTER_STATE_IDLE);
        assert!(idle.accepting);

        let holding =
            parse_printer_line("printer Hold is holding new jobs.  enabled since now").unwrap();
        assert!(!holding.accepting);

        let stopped =
            parse_printer_line("printer Photo disabled since Tue Nov 10 09:00:00 2026 -").unwrap();
        assert_eq!(stopped.state, PRINTER_STATE_STOPPED);

        assert_eq!(
            parse_alerts_line("\tAlerts: media-low-warning toner-low-warning"),
            Some("media-low-warning,toner-low-warning".to_string())
        );
        assert_eq!(parse_alerts_line("\tAlerts: none"), None);

        let job = parse_job_line(
            "my-printer-15           daniel            1024   Tue Nov 10 11:00:00 2026",
        )
        .unwrap();
        assert_eq!(job.printer, "my-printer");
        assert_eq!(job.id, 15);
        assert_eq!(job.created_display, "Tue Nov 10 11:00:00 2026");
    }

    #[test]
    fn builds_a_queue_from_lpstat_text() {
        let destinations = "Office_Laser\nPhoto\n";
        let printers = "\
printer Office_Laser is idle.  enabled since Tue Nov 10 10:00:00 2026
\tAlerts: media-low-warning
printer Photo disabled since Tue Nov 10 09:00:00 2026 -
\treason unknown
";
        let jobs = "\
Office_Laser-4          daniel            2048   Tue Nov 10 11:00:00 2026
";
        let queue = queue_from_outputs(destinations, printers, jobs, "");
        assert!(queue.error.is_none());
        assert_eq!(queue.printers.len(), 2);
        let office = &queue.printers["Office_Laser"];
        assert_eq!(office.display_name, "Office Laser");
        assert_eq!(office.state_reasons, "media-low-warning");
        assert_eq!(
            office.jobs.get(&4).unwrap().created_display,
            "Tue Nov 10 11:00:00 2026"
        );
        assert_eq!(queue.printers["Photo"].state, PRINTER_STATE_STOPPED);
    }

    #[test]
    fn signals_update_and_remove_jobs() {
        let mut queue = Queue::empty();
        let changed = queue.handle_signal(CupsSignal::JobUpsert {
            printer: "Office".to_string(),
            printer_uri: "ipp://localhost/printers/Office".to_string(),
            printer_state: PRINTER_STATE_IDLE,
            printer_reasons: "none".to_string(),
            accepting: true,
            job_id: 4,
            job_state: 3,
            job_name: "report.pdf".to_string(),
        });
        assert_eq!(changed, ApplyResult::Changed);
        assert_eq!(queue.printers["Office"].jobs[&4].name, "report.pdf");

        let removed = queue.handle_signal(CupsSignal::JobUpsert {
            printer: "Office".to_string(),
            printer_uri: String::new(),
            printer_state: PRINTER_STATE_IDLE,
            printer_reasons: "none".to_string(),
            accepting: true,
            job_id: 4,
            job_state: JOB_STATE_CANCELED,
            job_name: String::new(),
        });
        assert_eq!(removed, ApplyResult::Changed);
        assert!(queue.printers["Office"].jobs.is_empty());
        assert_eq!(
            queue.printers["Office"].uri,
            "ipp://localhost/printers/Office"
        );

        queue.handle_signal(CupsSignal::ServerStopped);
        assert!(queue.printers.is_empty());
    }

    #[test]
    fn hides_dns_sd_duplicate_of_installed_queue() {
        let destinations = "\
Officejet5740
HP_Officejet_5740_series_E156B4
";
        let printers = "\
printer Officejet5740 is idle.  enabled since Sat Nov 10 10:00:00 2026
";
        let devices = "\
device for Officejet5740: ipp://192.168.0.22/ipp/print
device for HP_Officejet_5740_series_E156B4: dnssd://HP%20Officejet%205740%20series._ipp._tcp.local/?uuid=1c852a4d-b800-1f08-abcd-ecb1d7e156b4
";
        let queue = queue_from_outputs(destinations, printers, "", devices);
        assert_eq!(queue.printers.len(), 1);
        assert!(queue.printers.contains_key("Officejet5740"));
        let officejet = &queue.printers["Officejet5740"];
        assert_eq!(officejet.uri, "ipp://192.168.0.22/ipp/print");
    }

    #[test]
    fn hides_discovered_only_destinations_when_installed_queues_exist() {
        let destinations = "\
Office_Laser
Discovered_Only_Printer
";
        let printers = "\
printer Office_Laser is idle.  enabled since Sat Nov 10 10:00:00 2026
";
        let devices = "\
device for Office_Laser: ipp://192.168.1.50/ipp/print
device for Discovered_Only_Printer: dnssd://Other._ipp._tcp.local/?uuid=aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee
";
        let names = queue_printer_names(destinations, printers, &parse_device_uri_map(devices));
        assert_eq!(names, vec!["Office_Laser".to_string()]);
        let queue = queue_from_outputs(destinations, printers, "", devices);
        assert_eq!(queue.printers.len(), 1);
        assert!(queue.printers.contains_key("Office_Laser"));
    }

    #[test]
    fn device_identities_match_uuid_and_host() {
        let dnssd =
            "dnssd://HP%20Officejet._ipp._tcp.local/?uuid=1C852A4D-B800-1F08-ABCD-ECB1D7E156B4";
        let ipp = "ipp://192.168.0.22/ipp/print";
        assert!(
            device_identities(dnssd)
                .iter()
                .any(|id| id == "1c852a4d-b800-1f08-abcd-ecb1d7e156b4")
        );
        assert!(device_identities(ipp).contains(&"192.168.0.22".to_string()));
        let map = parse_device_uri_map(
            "device for A: ipp://192.168.0.22/ipp/print\ndevice for B: dnssd://x/?uuid=1c852a4d-b800-1f08-abcd-ecb1d7e156b4\n",
        );
        assert_eq!(map.len(), 2);
        assert_eq!(map["A"], "ipp://192.168.0.22/ipp/print");
    }

    #[test]
    fn printer_upsert_does_not_readd_filtered_dns_sd_destination() {
        let destinations = "\
Officejet5740
HP_Officejet_5740_series_E156B4
";
        let printers = "\
printer Officejet5740 is idle.  enabled since Sat Nov 10 10:00:00 2026
";
        let devices = "\
device for Officejet5740: ipp://192.168.0.22/ipp/print
device for HP_Officejet_5740_series_E156B4: dnssd://HP%20Officejet%205740%20series._ipp._tcp.local/?uuid=1c852a4d-b800-1f08-abcd-ecb1d7e156b4
";
        let mut queue = queue_from_outputs(destinations, printers, "", devices);
        assert_eq!(queue.printers.len(), 1);

        let result = queue.handle_signal(CupsSignal::PrinterUpsert {
            name: "HP_Officejet_5740_series_E156B4".to_string(),
            uri: "dnssd://HP%20Officejet%205740%20series._ipp._tcp.local/?uuid=1c852a4d-b800-1f08-abcd-ecb1d7e156b4".to_string(),
            state: PRINTER_STATE_IDLE,
            reasons: "none".to_string(),
            accepting: true,
            newly_added: false,
        });
        assert_eq!(result, ApplyResult::Unchanged);
        assert_eq!(queue.printers.len(), 1);
        assert!(
            !queue
                .printers
                .contains_key("HP_Officejet_5740_series_E156B4")
        );
    }

    #[test]
    fn job_on_filtered_destination_maps_to_installed_queue() {
        let destinations = "\
Officejet5740
HP_Officejet_5740_series_E156B4
";
        let printers = "\
printer Officejet5740 is idle.  enabled since Sat Nov 10 10:00:00 2026
";
        let devices = "\
device for Officejet5740: ipp://192.168.0.22/ipp/print?uuid=1c852a4d-b800-1f08-abcd-ecb1d7e156b4
device for HP_Officejet_5740_series_E156B4: dnssd://HP%20Officejet%205740%20series._ipp._tcp.local/?uuid=1c852a4d-b800-1f08-abcd-ecb1d7e156b4
";
        let jobs = "\
HP_Officejet_5740_series_E156B4-2          daniel            1024   Tue Nov 10 11:00:00 2026
";
        let queue = queue_from_outputs(destinations, printers, jobs, devices);
        assert_eq!(queue.printers.len(), 1);
        assert!(queue.printers.contains_key("Officejet5740"));
        assert_eq!(queue.printers["Officejet5740"].jobs.len(), 1);
        assert!(queue.printers["Officejet5740"].jobs.contains_key(&2));

        let mut queue = queue;
        let result = queue.handle_signal(CupsSignal::JobUpsert {
            printer: "HP_Officejet_5740_series_E156B4".to_string(),
            printer_uri: "dnssd://HP%20Officejet%205740%20series._ipp._tcp.local/?uuid=1c852a4d-b800-1f08-abcd-ecb1d7e156b4".to_string(),
            printer_state: PRINTER_STATE_IDLE,
            printer_reasons: "none".to_string(),
            accepting: true,
            job_id: 3,
            job_state: 3,
            job_name: "scan.pdf".to_string(),
        });
        assert_eq!(result, ApplyResult::Changed);
        assert_eq!(queue.printers.len(), 1);
        assert_eq!(queue.printers["Officejet5740"].jobs[&3].name, "scan.pdf");
    }

    #[test]
    fn parse_device_uri_map_handles_colon_in_queue_name() {
        let map = parse_device_uri_map("device for Lab: Color: ipp://192.168.1.10/ipp/print\n");
        assert_eq!(map.len(), 1);
        assert_eq!(map["Lab: Color"], "ipp://192.168.1.10/ipp/print");
    }

    #[test]
    fn parse_device_uri_map_tolerates_empty_and_partial_output() {
        assert!(parse_device_uri_map("").is_empty());
        assert!(parse_device_uri_map("no devices here\n").is_empty());
        let map =
            parse_device_uri_map("device for A: ipp://host/a\nmalformed line\n device for B: \n");
        assert_eq!(map.len(), 1);
        assert_eq!(map["A"], "ipp://host/a");
    }

    #[test]
    fn send_to_front_keeps_the_printing_job_first() {
        let mut jobs = jobs_with_states(&[JOB_STATE_PROCESSING, 3, 3]);
        send_job_to_front(&mut jobs, 3);
        let ids: Vec<u32> = jobs.keys().copied().collect();
        assert_eq!(ids, vec![1, 3, 2]);

        let mut queued = jobs_with_states(&[3, 3]);
        send_job_to_front(&mut queued, 2);
        let ids: Vec<u32> = queued.keys().copied().collect();
        assert_eq!(ids, vec![2, 1]);
    }
}
