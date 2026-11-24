use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;

use crate::html::build_configured_document;
use crate::system;
use crate::types::*;

pub const HOST: &str = "127.0.0.1";
pub const PORT: u16 = 18119;
pub const WS_PATH: &str = "/ws";
pub const SYSTEM_DEFAULT_KEY: &str = "__system_default__";
pub const MAX_JOBS: usize = 50;

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct PersistedState {
    active_printer_name: String,
    locale: Locale,
}

#[derive(Default)]
struct Inner {
    configs: HashMap<String, PrinterConfig>,
    jobs: Vec<PrintJob>,
    state: PersistedState,
    printers: Vec<PrinterInfo>,
    notice: Option<(bool, String, std::time::Instant)>,
}

fn app_data_dir() -> PathBuf {
    let dir = dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("report-client");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Take the process-wide instance lock. When another instance already holds it,
/// ask that instance to show its window and return `None`.
pub fn acquire_single_instance() -> Option<std::fs::File> {
    let dir = app_data_dir();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("instance.lock"))
        .ok()?;
    match file.try_lock() {
        Ok(()) => Some(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            let _ = std::fs::write(dir.join("show.flag"), b"1");
            None
        }
        // If locking is unsupported, don't block startup.
        Err(_) => Some(file),
    }
}

pub struct Core {
    inner: Mutex<Inner>,
    version: AtomicU64,
    print_gate: tokio::sync::Mutex<()>,
    data_dir: PathBuf,
    pub rt: Handle,
}

impl Core {
    pub fn new(rt: Handle) -> Arc<Self> {
        let data_dir = app_data_dir();
        let configs = read_json(&data_dir.join("printer-configs.json")).unwrap_or_default();
        let state = read_json(&data_dir.join("app-state.json")).unwrap_or_default();
        Arc::new(Self {
            inner: Mutex::new(Inner { configs, jobs: Vec::new(), state, printers: Vec::new(), notice: None }),
            version: AtomicU64::new(1),
            print_gate: tokio::sync::Mutex::new(()),
            data_dir,
            rt,
        })
    }

    /// Monotonic counter the UI polls to know when to redraw.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    /// True once if another launch asked this instance to show its window.
    pub fn take_show_request(&self) -> bool {
        std::fs::remove_file(self.data_dir.join("show.flag")).is_ok()
    }

    pub fn locale(&self) -> Locale {
        self.inner.lock().unwrap().state.locale
    }

    pub fn set_locale(&self, locale: Locale) {
        let mut inner = self.inner.lock().unwrap();
        inner.state.locale = locale;
        self.save_state(&inner);
        drop(inner);
        self.bump();
    }

    pub fn active_printer(&self) -> String {
        self.inner.lock().unwrap().state.active_printer_name.clone()
    }

    pub fn set_active_printer(&self, name: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.state.active_printer_name = name.to_string();
        self.save_state(&inner);
        drop(inner);
        self.bump();
    }

    pub fn config_for(&self, printer: &str) -> PrinterConfig {
        let inner = self.inner.lock().unwrap();
        let key = if printer.is_empty() { SYSTEM_DEFAULT_KEY } else { printer };
        inner.configs.get(key).cloned().unwrap_or_default().sanitized()
    }

    pub fn save_config(&self, printer: &str, config: PrinterConfig) {
        let key = if printer.is_empty() { SYSTEM_DEFAULT_KEY } else { printer };
        let mut inner = self.inner.lock().unwrap();
        inner.configs.insert(key.to_string(), config.sanitized());
        self.save_configs(&inner);
        drop(inner);
        self.bump();
    }

    pub fn delete_config(&self, printer: &str) {
        let key = if printer.is_empty() { SYSTEM_DEFAULT_KEY } else { printer };
        let mut inner = self.inner.lock().unwrap();
        inner.configs.remove(key);
        self.save_configs(&inner);
        drop(inner);
        self.bump();
    }

    pub fn printers(&self) -> Vec<PrinterInfo> {
        self.inner.lock().unwrap().printers.clone()
    }

    pub async fn refresh_printers(&self) {
        let printers = system::list_printers().await;
        let mut inner = self.inner.lock().unwrap();
        let changed = serde_json::to_string(&inner.printers).ok() != serde_json::to_string(&printers).ok();
        inner.printers = printers;
        drop(inner);
        if changed {
            self.bump();
        }
    }

    pub fn set_notice(&self, ok: bool, message: impl Into<String>) {
        self.inner.lock().unwrap().notice = Some((ok, message.into(), std::time::Instant::now()));
        self.bump();
    }

    /// The latest notice, if it is still fresh enough to show.
    pub fn notice(&self) -> Option<(bool, String)> {
        let inner = self.inner.lock().unwrap();
        inner
            .notice
            .as_ref()
            .filter(|(_, _, at)| at.elapsed() < Duration::from_secs(6))
            .map(|(ok, msg, _)| (*ok, msg.clone()))
    }

    pub fn jobs(&self) -> Vec<PrintJob> {
        self.inner.lock().unwrap().jobs.clone()
    }

    pub fn job(&self, id: &str) -> Option<PrintJob> {
        self.inner.lock().unwrap().jobs.iter().find(|j| j.id == id).cloned()
    }

    pub fn server_info(&self) -> serde_json::Value {
        serde_json::json!({
            "host": HOST,
            "port": PORT,
            "baseUrl": format!("http://{HOST}:{PORT}"),
            "wsUrl": format!("ws://{HOST}:{PORT}{WS_PATH}"),
            "bridgeReady": true,
            "recentJobCount": self.inner.lock().unwrap().jobs.len(),
        })
    }

    pub fn export_payload(&self) -> serde_json::Value {
        let inner = self.inner.lock().unwrap();
        serde_json::json!({
            "exportedAt": Utc::now().to_rfc3339(),
            "appVersion": env!("CARGO_PKG_VERSION"),
            "activePrinterName": inner.state.active_printer_name,
            "locale": inner.state.locale,
            "recentJobCount": inner.jobs.len(),
            "jobs": inner.jobs,
        })
    }

    pub fn export_jobs_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(&self.export_payload()).unwrap_or_default())
    }

    pub async fn ensure_active_printer(&self) {
        if !self.active_printer().is_empty() {
            return;
        }
        if let Some(default) = system::list_printers().await.into_iter().find(|p| p.is_default) {
            self.set_active_printer(&default.name);
        }
    }

    fn remember_job(&self, job: PrintJob) {
        let mut inner = self.inner.lock().unwrap();
        inner.jobs.insert(0, job);
        inner.jobs.truncate(MAX_JOBS);
        drop(inner);
        self.bump();
    }

    fn update_job(&self, id: &str, f: impl FnOnce(&mut PrintJob)) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(job) = inner.jobs.iter_mut().find(|j| j.id == id) {
            f(job);
            job.updated_at = Utc::now().to_rfc3339();
        }
        drop(inner);
        self.bump();
    }

    fn fail_job(&self, id: &str, printer: Option<&str>, result: &PrintResult) {
        self.update_job(id, |job| {
            if let Some(p) = printer {
                job.printer_name = p.to_string();
            }
            job.status = JobStatus::Failed;
            job.success = false;
            job.code = result.code;
            job.stage = result.stage;
            job.message = result.message.clone();
        });
    }

    fn save_configs(&self, inner: &Inner) {
        write_json(&self.data_dir.join("printer-configs.json"), &inner.configs);
    }

    fn save_state(&self, inner: &Inner) {
        write_json(&self.data_dir.join("app-state.json"), &inner.state);
    }

    pub async fn print_html(&self, payload: PrintPayload, source: JobSource) -> PrintResult {
        let id = format!("{}-{:06x}", Utc::now().timestamp_millis(), rand_suffix());
        let requested = payload.requested_printer();
        let title = payload.title_or_default();
        let initial_config = self.config_for(&requested);
        let now = Utc::now().to_rfc3339();
        self.remember_job(PrintJob {
            id: id.clone(),
            title: title.clone(),
            printer_name: requested.clone(),
            success: false,
            status: JobStatus::Received,
            source,
            created_at: now.clone(),
            updated_at: now,
            config_summary: initial_config.summary(),
            applied_config: initial_config,
            replay_payload: Some(PrintPayload {
                html: payload.html.clone(),
                content: payload.content.clone(),
                printer_name: Some(requested.clone()),
                device_name: None,
                title: payload.title.clone(),
                silent: Some(payload.silent != Some(false)),
            }),
            message: None,
            code: None,
            stage: None,
        });

        let html = payload.html.clone().or(payload.content.clone()).unwrap_or_default();
        if html.is_empty() {
            let r = PrintResult::fail(ErrorCode::ParamInvalid, Stage::Render, "打印内容为空");
            self.fail_job(&id, None, &r);
            return r;
        }

        let printers = system::list_printers().await;
        if !requested.is_empty() && !printers.iter().any(|p| p.name == requested) {
            let r = PrintResult::fail(ErrorCode::PrinterNotFound, Stage::PrintSubmit, format!("未找到指定打印机: {requested}"));
            self.fail_job(&id, Some(&requested), &r);
            return r;
        }
        let device = system::preferred_printer_name(&printers, &requested);
        if device.is_empty() && printers.is_empty() {
            let r = PrintResult::fail(ErrorCode::PrinterNotFound, Stage::PrintSubmit, "未检测到可用打印机");
            self.fail_job(&id, None, &r);
            return r;
        }
        if let Some(printer) = printers.iter().find(|p| p.name == device) {
            let status = system::normalize_status(&printer.status);
            if !status.available {
                let suffix = if status.raw.is_empty() { String::new() } else { format!(" ({})", status.raw) };
                let mut r = PrintResult::fail(ErrorCode::PrinterUnavailable, Stage::PrintSubmit, format!("打印机不可用: {device}{suffix}"));
                r.printer_name = Some(device.clone());
                self.fail_job(&id, Some(&device), &r);
                return r;
            }
        }

        let config = self.config_for(&device);
        self.update_job(&id, |job| {
            job.printer_name = device.clone();
            job.status = JobStatus::Rendering;
            job.config_summary = config.summary();
            job.applied_config = config.clone();
        });

        let document = build_configured_document(&html, &config, &device, &title);
        let work_dir = std::env::temp_dir().join(format!("report-client-{id}"));
        let gate = self.print_gate.lock().await;
        let outcome = Self::render_and_submit(&work_dir, &document, &device, &title, &config).await;
        drop(gate);
        let _ = std::fs::remove_dir_all(&work_dir);
        let (mut result, baseline) = match outcome {
            Ok(baseline) => (PrintResult { success: true, printer_name: Some(device.clone()), ..Default::default() }, baseline),
            Err((code, stage, message)) => {
                let mut r = PrintResult::fail(code, stage, message);
                r.printer_name = Some(device.clone());
                self.fail_job(&id, Some(&device), &r);
                return r;
            }
        };

        self.update_job(&id, |job| {
            job.status = JobStatus::Submitted;
            job.success = true;
        });

        if !device.is_empty() {
            self.follow_queue(&id, &device, baseline).await;
        }
        result.printer_name = Some(device);
        result
    }

    async fn render_and_submit(
        work_dir: &std::path::Path,
        document: &str,
        device: &str,
        title: &str,
        config: &PrinterConfig,
    ) -> Result<Option<usize>, (ErrorCode, Stage, String)> {
        let render_err = |m: String| (ErrorCode::RenderFailed, Stage::Render, m);
        std::fs::create_dir_all(work_dir).map_err(|e| render_err(e.to_string()))?;
        let html_path = work_dir.join("document.html");
        let pdf_path = work_dir.join("document.pdf");
        std::fs::write(&html_path, document).map_err(|e| render_err(e.to_string()))?;
        system::render_pdf(&html_path, &pdf_path).await.map_err(render_err)?;

        let baseline = system::queue_count(device).await;
        system::submit_pdf(&pdf_path, device, title, config)
            .await
            .map_err(|m| (system::map_failure_to_code(&m), Stage::PrintCallback, if m.is_empty() { "打印任务提交失败".to_string() } else { m }))?;
        Ok(baseline)
    }

    async fn follow_queue(&self, id: &str, device: &str, baseline: Option<usize>) {
        if system::wait_for_queue_completion(device, baseline, Duration::from_secs(60), Duration::from_secs(2)).await {
            self.update_job(id, |job| {
                job.success = true;
                job.status = JobStatus::Completed;
                job.message = Some("打印任务已从系统打印队列完成".into());
            });
            return;
        }
        let runtime = system::list_printers().await.into_iter().find(|p| p.name == device);
        let count = system::queue_count(device).await;
        let blocked = matches!((baseline, count), (Some(b), Some(c)) if c > b);
        let unavailable = runtime.as_ref().is_some_and(|p| !system::normalize_status(&p.status).available);
        let message = if unavailable {
            let status = runtime.map(|p| p.status).unwrap_or_default();
            format!("打印任务未完成,打印机当前不可用: {device}{}", if status.is_empty() { String::new() } else { format!(" ({status})") })
        } else if blocked {
            format!("打印任务未完成,仍滞留在系统打印队列: {device}")
        } else {
            "打印任务未完成,未在超时时间内确认打印队列完成".to_string()
        };
        self.update_job(id, |job| {
            job.success = false;
            job.status = JobStatus::Failed;
            job.code = Some(ErrorCode::PrintTimeout);
            job.stage = Some(Stage::PrintCallback);
            job.message = Some(message);
        });
    }
}

fn rand_suffix() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new().build_hasher().finish() as u32 & 0xff_ffff
}

fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_json<T: Serialize>(path: &std::path::Path, value: &T) {
    if let Ok(text) = serde_json::to_string_pretty(value) {
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("write {} failed: {e}", path.display());
        }
    }
}
