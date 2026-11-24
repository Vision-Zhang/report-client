use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::types::{ErrorCode, Locale, PrinterConfig, PrinterInfo, PrintResult, Stage};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusKind {
    Ready,
    Busy,
    Unavailable,
    Unknown,
}

pub struct StatusInfo {
    pub raw: String,
    pub available: bool,
    pub kind: StatusKind,
}

pub fn normalize_status(status: &str) -> StatusInfo {
    let raw = status.trim().to_string();
    let lower = raw.to_lowercase();
    let (available, kind) = if lower.is_empty() {
        (false, StatusKind::Unknown)
    } else if ["offline", "error", "unavailable", "stopped", "disabled", "paused"].iter().any(|k| lower.contains(k)) {
        (false, StatusKind::Unavailable)
    } else if lower.contains("printing") || lower.contains("busy") {
        (true, StatusKind::Busy)
    } else if lower.contains("idle") || lower.contains("ready") {
        (true, StatusKind::Ready)
    } else {
        (true, StatusKind::Unknown)
    };
    StatusInfo { raw, available, kind }
}

pub fn is_virtual_export_printer(printer: &PrinterInfo) -> bool {
    let text = format!("{} {}", printer.name, printer.display_name).to_lowercase();
    ["microsoft print to pdf", "print to pdf", "xps document writer", "onenote", "fax"]
        .iter()
        .any(|k| text.contains(k))
}

pub fn preferred_printer_name(printers: &[PrinterInfo], requested: &str) -> String {
    if !requested.is_empty() && printers.iter().any(|p| p.name == requested) {
        return requested.to_string();
    }
    printers.iter().find(|p| p.is_default).map(|p| p.name.clone()).unwrap_or_default()
}

async fn run(program: &str, args: &[&str]) -> Result<(String, String), String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("{program}: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if output.status.success() {
        Ok((stdout, stderr))
    } else {
        Err(if stderr.trim().is_empty() { stdout } else { stderr }.trim().to_string())
    }
}

async fn powershell(script: &str) -> Result<String, String> {
    let wrapped = format!("[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; {script}");
    run("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", &wrapped])
        .await
        .map(|(out, _)| out)
}

fn ps_quote(value: &str) -> String {
    value.replace('\'', "''")
}

pub async fn list_printers() -> Vec<PrinterInfo> {
    if cfg!(windows) {
        list_printers_windows().await
    } else {
        list_printers_unix().await
    }
}

async fn list_printers_unix() -> Vec<PrinterInfo> {
    let Ok((stdout, _)) = run("lpstat", &["-p", "-d"]).await else {
        return Vec::new();
    };
    let mut default_name = String::new();
    let mut found: Vec<(String, String)> = Vec::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("system default destination:") {
            default_name = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("printer ") {
            let mut it = rest.splitn(2, ' ');
            let name = it.next().unwrap_or("").to_string();
            let tail = it.next().unwrap_or("");
            let status = tail
                .trim_start_matches("is ")
                .split(['.', '\n'])
                .next()
                .unwrap_or("idle")
                .trim()
                .to_string();
            if !name.is_empty() {
                found.push((name, status));
            }
        }
    }
    let mut printers = Vec::new();
    for (name, status) in found {
        let paper = run("lpoptions", &["-p", &name]).await.ok().and_then(|(out, _)| {
            out.split_whitespace()
                .find_map(|kv| kv.strip_prefix("media=").map(|v| v.to_string()))
        });
        printers.push(PrinterInfo {
            display_name: name.replace('_', " "),
            is_default: name == default_name,
            status: if status.is_empty() { "idle".into() } else { status },
            paper_size_label: paper,
            name,
        });
    }
    printers
}

async fn list_printers_windows() -> Vec<PrinterInfo> {
    let script = "Get-CimInstance -ClassName Win32_Printer | Select-Object Name,Default,PrinterStatus,WorkOffline | ConvertTo-Json -Compress";
    let Ok(out) = powershell(script).await else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(out.trim()) else {
        return Vec::new();
    };
    let items = match value {
        serde_json::Value::Array(items) => items,
        obj @ serde_json::Value::Object(_) => vec![obj],
        _ => Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            let name = item.get("Name")?.as_str()?.to_string();
            let offline = item.get("WorkOffline").and_then(|v| v.as_bool()).unwrap_or(false);
            let status = if offline {
                "offline"
            } else {
                match item.get("PrinterStatus").and_then(|v| v.as_u64()).unwrap_or(0) {
                    3 => "idle",
                    4 => "printing",
                    7 => "offline",
                    1 | 2 | 5 | 6 => "idle",
                    _ => "unknown",
                }
            };
            Some(PrinterInfo {
                display_name: name.clone(),
                is_default: item.get("Default").and_then(|v| v.as_bool()).unwrap_or(false),
                status: status.into(),
                paper_size_label: None,
                name,
            })
        })
        .collect()
}

pub async fn queue_count(printer: &str) -> Option<usize> {
    if printer.is_empty() {
        return None;
    }
    if cfg!(windows) {
        let script = format!("(Get-PrintJob -PrinterName '{}' -ErrorAction SilentlyContinue | Measure-Object).Count", ps_quote(printer));
        powershell(&script).await.ok()?.trim().parse().ok()
    } else {
        let (out, _) = run("lpstat", &["-o", printer]).await.ok()?;
        Some(out.lines().filter(|l| !l.trim().is_empty()).count())
    }
}

pub async fn wait_for_queue_completion(printer: &str, baseline: Option<usize>, timeout: Duration, interval: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let (Some(current), Some(base)) = (queue_count(printer).await, baseline) {
            if current <= base {
                return true;
            }
        }
        tokio::time::sleep(interval).await;
    }
    false
}

pub fn map_failure_to_code(reason: &str) -> ErrorCode {
    let r = reason.to_lowercase();
    if r.contains("printer") || r.contains("device") || r.contains("destination") {
        ErrorCode::PrinterNotFound
    } else if r.contains("timeout") || r.contains("timed out") {
        ErrorCode::PrintTimeout
    } else {
        ErrorCode::Unknown
    }
}

/// Locate a Chromium-based browser used to render HTML into a PDF.
pub fn find_browser() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("REPORT_CLIENT_BROWSER") {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Some(p);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        for app in ["Google Chrome", "Microsoft Edge", "Chromium", "Brave Browser"] {
            candidates.push(format!("/Applications/{app}.app/Contents/MacOS/{app}").into());
        }
    } else if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
            if let Ok(base) = std::env::var(var) {
                candidates.push(Path::new(&base).join("Microsoft/Edge/Application/msedge.exe"));
                candidates.push(Path::new(&base).join("Google/Chrome/Application/chrome.exe"));
            }
        }
    } else {
        for name in ["google-chrome", "chromium", "chromium-browser", "microsoft-edge"] {
            for dir in ["/usr/bin", "/usr/local/bin", "/snap/bin"] {
                candidates.push(Path::new(dir).join(name));
            }
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

pub async fn render_pdf(html_path: &Path, pdf_path: &Path) -> Result<(), String> {
    let browser = find_browser().ok_or("未找到 Chrome / Edge / Chromium,可通过环境变量 REPORT_CLIENT_BROWSER 指定浏览器路径")?;
    let profile = pdf_path.with_extension("profile");
    let url = format!("file://{}{}", if cfg!(windows) { "/" } else { "" }, html_path.to_string_lossy().replace('\\', "/"));
    let mut cmd = Command::new(browser);
    cmd.args([
        "--headless=new",
        "--disable-gpu",
        "--no-sandbox",
        "--no-first-run",
        "--no-default-browser-check",
        "--no-pdf-header-footer",
        "--run-all-compositor-stages-before-draw",
        "--virtual-time-budget=15000",
    ])
    .arg(format!("--user-data-dir={}", profile.display()))
    .arg(format!("--print-to-pdf={}", pdf_path.display()))
    .arg(url)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(60), cmd.output()).await;
    let _ = std::fs::remove_dir_all(&profile);
    match result {
        Err(_) => Err("HTML 渲染超时".into()),
        Ok(Err(e)) => Err(format!("无法启动浏览器: {e}")),
        Ok(Ok(out)) => {
            if pdf_path.exists() {
                Ok(())
            } else {
                Err(format!("HTML 渲染失败: {}", String::from_utf8_lossy(&out.stderr).trim()))
            }
        }
    }
}

fn find_sumatra() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("REPORT_CLIENT_SUMATRA") {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Some(p);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("SumatraPDF.exe"));
        }
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
        if let Ok(base) = std::env::var(var) {
            candidates.push(Path::new(&base).join("SumatraPDF/SumatraPDF.exe"));
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Submit a PDF to the system spooler. Returns the printer the job went to.
pub async fn submit_pdf(pdf: &Path, printer: &str, title: &str, config: &PrinterConfig) -> Result<(), String> {
    if cfg!(windows) {
        let sumatra = find_sumatra().ok_or("未找到 SumatraPDF.exe,请安装或通过 REPORT_CLIENT_SUMATRA 指定路径")?;
        let settings = format!("fit,{}", if config.duplex_mode { "duplexlong" } else { "simplex" });
        let mut args: Vec<String> = Vec::new();
        if printer.is_empty() {
            args.push("-print-to-default".into());
        } else {
            args.push("-print-to".into());
            args.push(printer.into());
        }
        args.extend(["-print-settings".into(), settings, "-silent".into(), pdf.to_string_lossy().into_owned()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run(&sumatra.to_string_lossy(), &refs).await.map(|_| ())
    } else {
        let pdf_str = pdf.to_string_lossy().into_owned();
        let sides = if config.duplex_mode { "sides=two-sided-long-edge" } else { "sides=one-sided" };
        let mut args = vec!["-t", title, "-o", "fit-to-page", "-o", sides];
        if !printer.is_empty() {
            args.extend(["-d", printer]);
        }
        args.push(&pdf_str);
        run("lp", &args).await.map(|_| ())
    }
}

pub async fn print_test_page(printer: &str, locale: Locale) -> PrintResult {
    let tr = |zh: &str, en: &str| if locale == Locale::EnUs { en.to_string() } else { zh.to_string() };
    let printers = list_printers().await;
    if !printer.is_empty() && !printers.iter().any(|p| p.name == printer) {
        return PrintResult::fail(ErrorCode::PrinterNotFound, Stage::PrintSubmit, tr("所选打印机已不可用。", "The selected printer is no longer available."));
    }
    let target = preferred_printer_name(&printers, printer);
    let Some(info) = printers.iter().find(|p| p.name == target) else {
        return PrintResult::fail(ErrorCode::PrinterNotFound, Stage::PrintSubmit, tr("未找到可用打印机。", "No available printer was found."));
    };
    if is_virtual_export_printer(info) {
        return PrintResult::fail(
            ErrorCode::PrinterUnavailable,
            Stage::PrintSubmit,
            tr("当前打印机是导出型虚拟打印机,已禁用系统测试页功能。", "The selected printer is a virtual export printer, so the system test page is disabled."),
        );
    }
    let outcome = if cfg!(windows) {
        let script = format!(
            "$p = Get-CimInstance -ClassName Win32_Printer | Where-Object {{ $_.Name -eq '{}' }} | Select-Object -First 1; if ($null -eq $p) {{ throw 'PRINTER_NOT_FOUND' }}; $r = Invoke-CimMethod -InputObject $p -MethodName PrintTestPage -ErrorAction Stop; if ($null -ne $r.ReturnValue -and [int]$r.ReturnValue -ne 0) {{ throw \"PRINT_TEST_PAGE_FAILED:$($r.ReturnValue)\" }}",
            ps_quote(&target)
        );
        powershell(&script).await.map(|_| ())
    } else {
        let template = [
            "/usr/share/cups/data/testprint",
            "/usr/share/cups/data/testprint.ps",
            "/usr/local/share/cups/data/testprint",
            "/opt/homebrew/share/cups/data/testprint",
        ]
        .iter()
        .find(|p| Path::new(p).exists());
        match template {
            None => {
                return PrintResult::fail(ErrorCode::RenderFailed, Stage::Render, tr("未找到系统测试页模板文件。", "The system test page template was not found."));
            }
            Some(path) => run("lp", &["-d", &target, "-t", &tr("系统测试页", "System Test Page"), path]).await.map(|_| ()),
        }
    };
    match outcome {
        Ok(()) => PrintResult { success: true, printer_name: Some(target), ..Default::default() },
        Err(e) => PrintResult::fail(
            ErrorCode::Unknown,
            Stage::PrintSubmit,
            if e.is_empty() { tr("发送系统测试页失败。", "Failed to print the system test page.") } else { e },
        ),
    }
}
