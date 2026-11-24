use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Locale {
    #[default]
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en-US")]
    EnUs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrinterInfo {
    pub name: String,
    pub display_name: String,
    pub is_default: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paper_size_label: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintPayload {
    pub html: Option<String>,
    pub content: Option<String>,
    pub printer_name: Option<String>,
    pub device_name: Option<String>,
    pub title: Option<String>,
    pub silent: Option<bool>,
}

impl PrintPayload {
    pub fn requested_printer(&self) -> String {
        self.printer_name
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| self.device_name.clone())
            .unwrap_or_default()
    }

    pub fn title_or_default(&self) -> String {
        self.title.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "打印".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Received,
    Rendering,
    Submitted,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    #[serde(rename = "REPORT_CLIENT_OFFLINE")]
    ClientOffline,
    #[serde(rename = "REPORT_PRINT_TIMEOUT")]
    PrintTimeout,
    #[serde(rename = "REPORT_PRINTER_NOT_FOUND")]
    PrinterNotFound,
    #[serde(rename = "REPORT_PRINTER_UNAVAILABLE")]
    PrinterUnavailable,
    #[serde(rename = "REPORT_RENDER_FAILED")]
    RenderFailed,
    #[serde(rename = "REPORT_PARAM_INVALID")]
    ParamInvalid,
    #[serde(rename = "REPORT_UNKNOWN")]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    WsConnect,
    Render,
    PrintSubmit,
    PrintCallback,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub printer_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
}

impl PrintResult {
    pub fn fail(code: ErrorCode, stage: Stage, message: impl Into<String>) -> Self {
        Self {
            success: false,
            printer_name: None,
            message: Some(message.into()),
            code: Some(code),
            stage: Some(stage),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    #[serde(rename = "RAW")]
    Raw,
    #[serde(rename = "TEXT")]
    Text,
    #[serde(rename = "IMAGE")]
    Image,
}

impl DataType {
    pub fn as_str(self) -> &'static str {
        match self {
            DataType::Raw => "RAW",
            DataType::Text => "TEXT",
            DataType::Image => "IMAGE",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PrinterConfig {
    pub print_top_logo: bool,
    pub top_logo_url: String,
    pub print_middle_logo: bool,
    pub middle_logo_url: String,
    pub print_bottom_logo: bool,
    pub bottom_logo_url: String,
    pub offset_x: f64,
    pub offset_y: f64,
    pub auto_paper_size: bool,
    pub auto_paper_direction: bool,
    pub dpi: u32,
    pub duplex_mode: bool,
    pub embedded_font: bool,
    pub data_type: DataType,
    pub high_quality_print: bool,
    pub rotate: f64,
    pub scale: f64,
    pub document_dithering: bool,
    pub force_no_white: bool,
}

impl Default for PrinterConfig {
    fn default() -> Self {
        Self {
            print_top_logo: false,
            top_logo_url: String::new(),
            print_middle_logo: false,
            middle_logo_url: String::new(),
            print_bottom_logo: false,
            bottom_logo_url: String::new(),
            offset_x: 0.0,
            offset_y: 0.0,
            auto_paper_size: true,
            auto_paper_direction: true,
            dpi: 203,
            duplex_mode: false,
            embedded_font: false,
            data_type: DataType::Raw,
            high_quality_print: false,
            rotate: 0.0,
            scale: 1.0,
            document_dithering: false,
            force_no_white: true,
        }
    }
}

impl PrinterConfig {
    /// Clamp values that would otherwise produce an unusable document.
    pub fn sanitized(mut self) -> Self {
        if !self.scale.is_finite() || self.scale <= 0.0 {
            self.scale = 1.0;
        }
        self.scale = self.scale.clamp(0.1, 10.0);
        if !self.rotate.is_finite() {
            self.rotate = 0.0;
        }
        if !self.offset_x.is_finite() {
            self.offset_x = 0.0;
        }
        if !self.offset_y.is_finite() {
            self.offset_y = 0.0;
        }
        self.dpi = self.dpi.clamp(72, 2400);
        self
    }

    pub fn summary(&self) -> String {
        let logos: String = [
            (self.print_top_logo, "T"),
            (self.print_middle_logo, "M"),
            (self.print_bottom_logo, "B"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, s)| *s)
        .collect();
        let mut parts = vec![
            self.data_type.as_str().to_string(),
            format!("{}dpi", self.dpi),
            format!("{}x", self.scale),
        ];
        if self.embedded_font {
            parts.push("Font".into());
        }
        if !logos.is_empty() {
            parts.push(format!("Logo:{logos}"));
        }
        if self.duplex_mode {
            parts.push("Duplex".into());
        }
        parts.join(" / ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobSource {
    Ui,
    Http,
    Ws,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintJob {
    pub id: String,
    pub title: String,
    pub printer_name: String,
    pub success: bool,
    pub status: JobStatus,
    pub source: JobSource,
    pub created_at: String,
    pub updated_at: String,
    pub config_summary: String,
    pub applied_config: PrinterConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replay_payload: Option<PrintPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<ErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
}
