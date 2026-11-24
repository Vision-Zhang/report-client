use regex::Regex;

use crate::types::{DataType, PrinterConfig};

pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn strip_html_to_text(html: &str) -> String {
    let steps: [(&str, &str); 5] = [
        (r"(?is)<style.*?</style>", " "),
        (r"(?is)<script.*?</script>", " "),
        (r"(?i)<br\s*/?>", "\n"),
        (r"(?i)</(p|div|tr|li|h[1-6])>", "\n"),
        (r"<[^>]+>", " "),
    ];
    let mut text = html.to_string();
    for (pattern, replacement) in steps {
        text = Regex::new(pattern).unwrap().replace_all(&text, replacement).into_owned();
    }
    let text = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("\r\n", "\n");
    Regex::new(r"\n{3,}").unwrap().replace_all(&text, "\n\n").trim().to_string()
}

fn inline_logo_data_url(text: &str) -> String {
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"132\" height=\"44\" viewBox=\"0 0 132 44\"><rect x=\"1\" y=\"1\" width=\"130\" height=\"42\" rx=\"8\" fill=\"#f4f7fc\" stroke=\"#4f6486\" stroke-width=\"2\"/><text x=\"66\" y=\"28\" text-anchor=\"middle\" font-size=\"18\" font-family=\"Arial, 'Microsoft YaHei', sans-serif\" fill=\"#31455f\" font-weight=\"700\">{}</text></svg>",
        escape_html(text)
    );
    let mut encoded = String::from("data:image/svg+xml;charset=utf-8,");
    for byte in svg.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn logo_layers(title: &str, printer_name: &str, config: &PrinterConfig) -> String {
    let text: String = if title.is_empty() { "PRINT".into() } else { title.chars().take(10).collect() };
    let label = escape_html(if printer_name.is_empty() { "default" } else { printer_name });
    let pick = |url: &str| if url.is_empty() { inline_logo_data_url(&text) } else { escape_html(url) };
    let layer = |on: bool, pos: &str, url: &str| {
        if on {
            format!(
                "<div class=\"report-printer-logo report-printer-logo--{pos}\"><img src=\"{}\" alt=\"{pos} logo\" /><span>{label}</span></div>",
                pick(url)
            )
        } else {
            String::new()
        }
    };
    [
        layer(config.print_top_logo, "top", &config.top_logo_url),
        layer(config.print_middle_logo, "middle", &config.middle_logo_url),
        layer(config.print_bottom_logo, "bottom", &config.bottom_logo_url),
    ]
    .concat()
}

fn wrap_text_document(text: &str, title: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\" /><title>{}</title></head><body><pre class=\"report-printer-text\">{}</pre></body></html>",
        escape_html(title),
        escape_html(text)
    )
}

fn extract_body_inner(html: &str) -> String {
    Regex::new(r"(?is)<body[^>]*>(.*)</body>")
        .unwrap()
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| html.to_string())
}

fn wrap_image_document(html: &str, title: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\" /><title>{}</title></head><body><div class=\"report-printer-image\">{}</div></body></html>",
        escape_html(title),
        extract_body_inner(html)
    )
}

fn preprocess(html: &str, config: &PrinterConfig, title: &str) -> String {
    let raw = if html.contains("<html") {
        html.to_string()
    } else {
        let safe_title = title.replace('<', "&lt;").replace('>', "&gt;");
        format!("<!DOCTYPE html><html><head><title>{safe_title}</title></head><body>{html}</body></html>")
    };
    match config.data_type {
        DataType::Text => wrap_text_document(&strip_html_to_text(&raw), title),
        DataType::Image => wrap_image_document(&raw, title),
        DataType::Raw => raw,
    }
}

pub fn build_configured_document(html: &str, config: &PrinterConfig, printer_name: &str, title: &str) -> String {
    let raw = preprocess(html, config, title);
    let transform = format!(
        "translate({}mm, {}mm) rotate({}deg) scale({})",
        config.offset_x, config.offset_y, config.rotate, config.scale
    );
    let margin = if config.force_no_white { "0" } else { "8mm" };
    let filter = if config.document_dithering { "contrast(1.02) saturate(0.98)" } else { "none" };
    let logos = logo_layers(title, printer_name, config);
    let font_family = if config.embedded_font {
        "font-family: \"PingFang SC\", \"Microsoft YaHei\", sans-serif;"
    } else {
        ""
    };
    let rendering = if config.high_quality_print { "auto" } else { "crisp-edges" };
    let rotate_norm = ((config.rotate % 360.0) + 360.0) % 360.0;
    let landscape = if !config.auto_paper_direction && (rotate_norm == 90.0 || rotate_norm == 270.0) { "size: landscape;" } else { "" };
    let injection = format!(
        r#"<style id="report-printer-config-style">
@page {{ margin: {margin}; {landscape} }}
html, body {{ margin: 0; padding: 0; background: #fff; {font_family} }}
body {{ transform-origin: top left; transform: {transform}; filter: {filter}; position: relative; -webkit-print-color-adjust: exact; print-color-adjust: exact; }}
img {{ image-rendering: {rendering}; }}
.report-printer-text {{ white-space: pre-wrap; word-break: break-word; margin: 0; padding: 12mm 10mm; font-size: 12pt; line-height: 1.65; }}
.report-printer-image {{ width: 100%; }}
.report-printer-logo {{ position: fixed; left: 8mm; display: flex; align-items: center; gap: 4mm; opacity: 0.9; z-index: 9999; }}
.report-printer-logo img {{ width: 35mm; height: auto; }}
.report-printer-logo span {{ font-size: 10pt; color: #40536f; font-weight: 600; letter-spacing: 0.08em; }}
.report-printer-logo--top {{ top: 6mm; }}
.report-printer-logo--middle {{ top: 50%; transform: translateY(-50%); }}
.report-printer-logo--bottom {{ bottom: 6mm; }}
</style>"#
    );

    let with_head = if raw.contains("</head>") {
        raw.replacen("</head>", &format!("{injection}</head>"), 1)
    } else {
        raw.replacen("<html>", &format!("<html><head>{injection}</head>"), 1)
    };
    with_head.replacen("</body>", &format!("{logos}</body>"), 1)
}
