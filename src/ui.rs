use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::*;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::*;
use gpui_kit::*;
use gpui_kit::prelude::FluentBuilder as _;

use crate::core::Core;
use crate::system;
use crate::tray::{Tray, TrayAction};
use crate::types::*;

struct Inputs {
    offset_x: Entity<InputState>,
    offset_y: Entity<InputState>,
    dpi: Entity<InputState>,
    rotate: Entity<InputState>,
    scale: Entity<InputState>,
    top_url: Entity<InputState>,
    middle_url: Entity<InputState>,
    bottom_url: Entity<InputState>,
}

pub struct ReportApp {
    core: Arc<Core>,
    tab: usize,
    seen_version: u64,
    notice_visible: bool,
    draft: PrinterConfig,
    draft_for: Option<String>,
    inputs: Inputs,
    tray: Option<Tray>,
    logo: Arc<Image>,
}

impl ReportApp {
    pub fn new(core: Arc<Core>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mk = |window: &mut Window, cx: &mut Context<Self>, placeholder: &str| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let inputs = Inputs {
            offset_x: mk(window, cx, "0"),
            offset_y: mk(window, cx, "0"),
            dpi: mk(window, cx, "203"),
            rotate: mk(window, cx, "0"),
            scale: mk(window, cx, "1"),
            top_url: mk(window, cx, "https://"),
            middle_url: mk(window, cx, "https://"),
            bottom_url: mk(window, cx, "https://"),
        };

        // Closing the window keeps the print service running in the background.
        let tray = Tray::new();
        let has_tray = tray.is_some();
        window.on_window_should_close(cx, move |window, cx| {
            if !has_tray {
                cx.quit();
                return true;
            }
            if cfg!(target_os = "macos") {
                cx.hide();
            } else {
                window.minimize_window();
            }
            false
        });

        let rt_core = core.clone();
        core.rt.spawn(async move {
            rt_core.ensure_active_printer().await;
            loop {
                rt_core.refresh_printers().await;
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });

        // Redraw whenever the backend reports a change.
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            let alive = this
                .update_in(cx, |this, window, cx| {
                    let action = this.tray.as_ref().and_then(Tray::poll);
                    let action = if this.core.take_show_request() { Some(TrayAction::Show) } else { action };
                    match action {
                        Some(TrayAction::Show) => {
                            window.activate_window();
                            cx.activate(true);
                        }
                        Some(TrayAction::Quit) => cx.quit(),
                        None => {}
                    }
                    let version = this.core.version();
                    let notice_visible = this.core.notice().is_some();
                    let active = this.core.active_printer();
                    if this.draft_for.as_deref() != Some(active.as_str()) {
                        this.load_draft(&active, window, cx);
                    }
                    if version != this.seen_version || notice_visible != this.notice_visible {
                        this.seen_version = version;
                        this.notice_visible = notice_visible;
                        cx.notify();
                    }
                })
                .is_ok();
            if !alive {
                break;
            }
        })
        .detach();

        Self {
            core,
            tab: 0,
            seen_version: 0,
            notice_visible: false,
            draft: PrinterConfig::default(),
            draft_for: None,
            inputs,
            tray,
            logo: Arc::new(Image::from_bytes(ImageFormat::Png, crate::brand::ICON_PNG.to_vec())),
        }
    }

    fn t(&self, zh: &'static str, en: &'static str) -> &'static str {
        if self.core.locale() == Locale::EnUs { en } else { zh }
    }

    fn load_draft(&mut self, printer: &str, window: &mut Window, cx: &mut Context<Self>) {
        let config = self.core.config_for(printer);
        let set = |state: &Entity<InputState>, value: String, window: &mut Window, cx: &mut Context<Self>| {
            state.update(cx, |s, cx| s.set_value(value, window, cx));
        };
        set(&self.inputs.offset_x, config.offset_x.to_string(), window, cx);
        set(&self.inputs.offset_y, config.offset_y.to_string(), window, cx);
        set(&self.inputs.dpi, config.dpi.to_string(), window, cx);
        set(&self.inputs.rotate, config.rotate.to_string(), window, cx);
        set(&self.inputs.scale, config.scale.to_string(), window, cx);
        set(&self.inputs.top_url, config.top_logo_url.clone(), window, cx);
        set(&self.inputs.middle_url, config.middle_logo_url.clone(), window, cx);
        set(&self.inputs.bottom_url, config.bottom_logo_url.clone(), window, cx);
        self.draft = config;
        self.draft_for = Some(printer.to_string());
        cx.notify();
    }

    fn collect_draft(&self, cx: &App) -> PrinterConfig {
        let mut c = self.draft.clone();
        let text = |s: &Entity<InputState>| s.read(cx).value().trim().to_string();
        c.offset_x = text(&self.inputs.offset_x).parse().unwrap_or(0.0);
        c.offset_y = text(&self.inputs.offset_y).parse().unwrap_or(0.0);
        c.dpi = text(&self.inputs.dpi).parse().unwrap_or(203);
        c.rotate = text(&self.inputs.rotate).parse().unwrap_or(0.0);
        c.scale = text(&self.inputs.scale).parse().unwrap_or(1.0);
        c.top_logo_url = text(&self.inputs.top_url);
        c.middle_logo_url = text(&self.inputs.middle_url);
        c.bottom_logo_url = text(&self.inputs.bottom_url);
        c.sanitized()
    }

    fn card(&self, title: &'static str, hint: Option<&'static str>, cx: &App) -> Div {
        let theme = cx.theme();
        v_flex()
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.group_box)
            .child(
                v_flex()
                    .gap_0p5()
                    .child(div().text_sm().font_semibold().child(title))
                    .when_some(hint, |d, h| d.child(div().text_xs().text_color(theme.muted_foreground).child(h))),
            )
    }

    fn page_header(&self, title: &'static str, subtitle: &'static str, actions: impl IntoElement, cx: &App) -> impl IntoElement {
        h_flex()
            .items_center()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .gap_1()
                    .child(div().text_xl().font_semibold().child(title))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child(subtitle)),
            )
            .child(actions)
    }

    fn empty_state(&self, text: &'static str, cx: &App) -> impl IntoElement {
        div()
            .py_10()
            .flex()
            .items_center()
            .justify_center()
            .rounded_lg()
            .border_1()
            .border_dashed()
            .border_color(cx.theme().border)
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text)
    }

    fn dot(color: Hsla) -> Div {
        div().size_2().rounded_full().bg(color)
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let en = self.core.locale() == Locale::EnUs;
        let items = [
            (0usize, self.t("打印机", "Printers")),
            (1, self.t("打印配置", "Settings")),
            (2, self.t("打印记录", "Jobs")),
        ];
        let info = self.core.server_info();
        let port = info["port"].as_u64().unwrap_or_default();
        v_flex()
            .w(px(210.))
            .h_full()
            .flex_shrink_0()
            .p_3()
            .gap_4()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .text_color(theme.sidebar_foreground)
            .child(
                h_flex()
                    .gap_2()
                    .px_2()
                    .pt_2()
                    .items_center()
                    .child(img(self.logo.clone()).size_8().rounded_lg())
                    .child(
                        v_flex()
                            .child(div().text_sm().font_semibold().child(self.t("报表打印", "Report Printer")))
                            .child(div().text_xs().text_color(theme.muted_foreground).child(concat!("v", env!("CARGO_PKG_VERSION")))),
                    ),
            )
            .child(v_flex().gap_1().children(items.into_iter().map(|(ix, label)| {
                let selected = self.tab == ix;
                h_flex()
                    .id(("nav", ix))
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .rounded_md()
                    .cursor_pointer()
                    .text_sm()
                    .when(selected, |d| d.bg(theme.sidebar_accent).text_color(theme.sidebar_accent_foreground).font_semibold())
                    .when(!selected, |d| d.hover(|h| h.bg(theme.sidebar_accent.opacity(0.5))))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = ix;
                        cx.notify();
                    }))
            })))
            .child(div().flex_1())
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .bg(theme.sidebar_accent.opacity(0.6))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Self::dot(theme.success))
                            .child(div().text_xs().font_semibold().child(self.t("服务运行中", "Service running"))),
                    )
                    .child(div().text_xs().text_color(theme.muted_foreground).child(format!("127.0.0.1:{port}"))),
            )
            .child(
                Button::new("locale")
                    .small()
                    .outline()
                    .label(if en { "切换为中文" } else { "Switch to English" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.core.set_locale(if en { Locale::ZhCn } else { Locale::EnUs });
                        cx.notify();
                    })),
            )
    }

    fn notice_bar(&self, cx: &App) -> Option<impl IntoElement> {
        let (ok, message) = self.core.notice()?;
        let color = if ok { cx.theme().success } else { cx.theme().danger };
        Some(
            div()
                .rounded_md()
                .border_1()
                .border_color(color.opacity(0.5))
                .bg(cx.theme().background)
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .bg(color.opacity(0.1))
                        .text_sm()
                        .text_color(color)
                        .child(Self::dot(color))
                        .child(message),
                ),
        )
    }

    fn printers_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let printers = self.core.printers();
        let active = self.core.active_printer();
        let theme = cx.theme().clone();
        let locale = self.core.locale();
        let mut list = v_flex().gap_3();
        if printers.is_empty() {
            list = list.child(self.empty_state(self.t("未检测到打印机", "No printers detected"), cx));
        }
        for (i, p) in printers.iter().enumerate() {
            let status = system::normalize_status(&p.status);
            let (color, tag) = match status.kind {
                system::StatusKind::Unavailable => (theme.danger, Tag::danger()),
                system::StatusKind::Busy => (theme.warning, Tag::warning()),
                system::StatusKind::Ready => (theme.success, Tag::success()),
                system::StatusKind::Unknown => (theme.muted_foreground, Tag::secondary()),
            };
            let name = p.name.clone();
            let name2 = p.name.clone();
            let is_active = p.name == active;
            let paper = p.paper_size_label.clone().unwrap_or_default();
            let mut sub = p.name.clone();
            if !paper.is_empty() {
                sub = format!("{sub}  ·  {paper}");
            }
            list = list.child(
                h_flex()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .items_center()
                    .rounded_lg()
                    .border_1()
                    .border_color(if is_active { theme.primary } else { theme.border })
                    .bg(if is_active { theme.primary.opacity(0.06) } else { theme.group_box })
                    .child(Self::dot(color))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(div().text_sm().font_semibold().child(p.display_name.clone()))
                                    .when(p.is_default, |d| d.child(Tag::secondary().child(self.t("系统默认", "Default"))))
                                    .when(is_active, |d| d.child(Tag::primary().child(self.t("当前", "Active")))),
                            )
                            .child(div().text_xs().text_color(theme.muted_foreground).truncate().child(sub)),
                    )
                    .child(tag.child(truncate_status(&p.status)))
                    .child(
                        Button::new(("use", i))
                            .small()
                            .outline()
                            .disabled(is_active)
                            .label(self.t("设为当前", "Use"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.core.set_active_printer(&name);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("test", i))
                            .small()
                            .outline()
                            .label(self.t("测试页", "Test page"))
                            .on_click(cx.listener(move |this, _, _, _| {
                                let core = this.core.clone();
                                let name = name2.clone();
                                this.core.rt.spawn(async move {
                                    let r = system::print_test_page(&name, locale).await;
                                    let msg = if r.success {
                                        if locale == Locale::EnUs { "Test page sent".to_string() } else { "测试页已发送".to_string() }
                                    } else {
                                        r.message.unwrap_or_default()
                                    };
                                    core.set_notice(r.success, msg);
                                });
                            })),
                    ),
            );
        }
        v_flex()
            .gap_5()
            .child(self.page_header(
                self.t("打印机", "Printers"),
                self.t("选择默认使用的打印机,并发送测试页确认连通", "Choose the active printer and send a test page"),
                Button::new("refresh").small().label(self.t("刷新", "Refresh")).on_click(cx.listener(|this, _, _, _| {
                    let core = this.core.clone();
                    this.core.rt.spawn(async move { core.refresh_printers().await });
                })),
                cx,
            ))
            .child(list)
    }

    fn toggle(
        &self,
        id: &'static str,
        label: &'static str,
        value: bool,
        cx: &mut Context<Self>,
        set: fn(&mut PrinterConfig, bool),
    ) -> impl IntoElement {
        div().w(px(230.)).child(Switch::new(id).checked(value).label(label).on_click(cx.listener(move |this, checked: &bool, _, cx| {
            set(&mut this.draft, *checked);
            cx.notify();
        })))
    }

    fn field(&self, label: &'static str, input: &Entity<InputState>, cx: &App) -> impl IntoElement {
        v_flex()
            .gap_1()
            .w(px(150.))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
            .child(Input::new(input))
    }

    fn logo_row(
        &self,
        id: &'static str,
        label: &'static str,
        value: bool,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
        set: fn(&mut PrinterConfig, bool),
    ) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(self.toggle(id, label, value, cx, set))
            .child(div().flex_1().child(Input::new(input)))
    }

    fn config_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.core.active_printer();
        let target = if active.is_empty() { self.t("系统默认", "System default").to_string() } else { active.clone() };
        let d = self.draft.clone();
        let types = [DataType::Raw, DataType::Text, DataType::Image];
        let save_for = active.clone();
        let reset_for = active.clone();
        let actions = h_flex()
            .gap_2()
            .child(
                Button::new("cfg-reset")
                    .small()
                    .outline()
                    .label(self.t("恢复默认", "Reset"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.core.delete_config(&reset_for);
                        this.load_draft(&reset_for, window, cx);
                        this.core.set_notice(true, if this.core.locale() == Locale::EnUs { "Reset to defaults" } else { "已恢复默认配置" });
                    })),
            )
            .child(
                Button::new("cfg-save")
                    .primary()
                    .small()
                    .label(self.t("保存", "Save"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let config = this.collect_draft(cx);
                        this.core.save_config(&save_for, config.clone());
                        this.draft = config;
                        this.core.set_notice(true, if this.core.locale() == Locale::EnUs { "Saved" } else { "配置已保存" });
                    })),
            );

        let type_card = self.card(self.t("数据类型", "Data type"), Some(self.t("RAW 原样打印;TEXT 仅文本;IMAGE 按图片排版", "RAW as-is; TEXT plain text; IMAGE as picture")), cx).child(
            h_flex().gap_2().children(types.map(|t| {
                let b = Button::new(t.as_str()).small().label(t.as_str());
                (if d.data_type == t { b.primary() } else { b.outline() }).on_click(cx.listener(move |this, _, _, cx| {
                    this.draft.data_type = t;
                    cx.notify();
                }))
            })),
        );

        let layout_card = self.card(self.t("版面调整", "Layout"), Some(self.t("偏移、缩放与分辨率", "Offset, scale and resolution")), cx).child(
            h_flex()
                .gap_4()
                .flex_wrap()
                .child(self.field(self.t("横向偏移 (mm)", "Offset X (mm)"), &self.inputs.offset_x, cx))
                .child(self.field(self.t("纵向偏移 (mm)", "Offset Y (mm)"), &self.inputs.offset_y, cx))
                .child(self.field("DPI", &self.inputs.dpi, cx))
                .child(self.field(self.t("旋转 (°)", "Rotate (°)"), &self.inputs.rotate, cx))
                .child(self.field(self.t("缩放", "Scale"), &self.inputs.scale, cx)),
        );

        let option_card = self.card(self.t("打印选项", "Options"), None, cx).child(
            h_flex()
                .gap_x_4()
                .gap_y_3()
                .flex_wrap()
                .child(self.toggle("auto-size", self.t("自动纸张大小", "Auto paper size"), d.auto_paper_size, cx, |c, v| c.auto_paper_size = v))
                .child(self.toggle("auto-dir", self.t("自动纸张方向", "Auto paper direction"), d.auto_paper_direction, cx, |c, v| c.auto_paper_direction = v))
                .child(self.toggle("duplex", self.t("双面打印", "Duplex"), d.duplex_mode, cx, |c, v| c.duplex_mode = v))
                .child(self.toggle("font", self.t("嵌入字体", "Embedded font"), d.embedded_font, cx, |c, v| c.embedded_font = v))
                .child(self.toggle("hq", self.t("高质量打印", "High quality"), d.high_quality_print, cx, |c, v| c.high_quality_print = v))
                .child(self.toggle("dither", self.t("文档抖动", "Dithering"), d.document_dithering, cx, |c, v| c.document_dithering = v))
                .child(self.toggle("nowhite", self.t("强制无白边", "No margins"), d.force_no_white, cx, |c, v| c.force_no_white = v)),
        );

        let logo_card = self.card(self.t("Logo", "Logos"), Some(self.t("留空则使用内置占位图", "Leave blank to use the built-in placeholder")), cx)
            .child(self.logo_row("logo-top", self.t("顶部 Logo", "Top logo"), d.print_top_logo, &self.inputs.top_url, cx, |c, v| c.print_top_logo = v))
            .child(self.logo_row("logo-mid", self.t("中部 Logo", "Middle logo"), d.print_middle_logo, &self.inputs.middle_url, cx, |c, v| c.print_middle_logo = v))
            .child(self.logo_row("logo-bot", self.t("底部 Logo", "Bottom logo"), d.print_bottom_logo, &self.inputs.bottom_url, cx, |c, v| c.print_bottom_logo = v));

        let subtitle: SharedString = format!("{}: {}", self.t("配置对象", "Target"), target).into();
        v_flex()
            .gap_5()
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(div().text_xl().font_semibold().child(self.t("打印配置", "Settings")))
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(subtitle)),
                    )
                    .child(actions),
            )
            .child(type_card)
            .child(layout_card)
            .child(option_card)
            .child(logo_card)
    }

    fn jobs_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let jobs = self.core.jobs();
        let theme = cx.theme().clone();
        let mut list = v_flex().gap_3();
        if jobs.is_empty() {
            list = list.child(self.empty_state(self.t("暂无打印记录", "No print jobs yet"), cx));
        }
        for (i, job) in jobs.iter().enumerate() {
            let (color, tag, text) = match job.status {
                JobStatus::Completed => (theme.success, Tag::success(), self.t("已完成", "Completed")),
                JobStatus::Submitted => (theme.info, Tag::info(), self.t("已提交", "Submitted")),
                JobStatus::Failed => (theme.danger, Tag::danger(), self.t("失败", "Failed")),
                JobStatus::Rendering => (theme.warning, Tag::warning(), self.t("渲染中", "Rendering")),
                JobStatus::Received => (theme.muted_foreground, Tag::secondary(), self.t("已接收", "Received")),
            };
            let when = chrono::DateTime::parse_from_rfc3339(&job.created_at)
                .map(|t| t.with_timezone(&chrono::Local).format("%m-%d %H:%M:%S").to_string())
                .unwrap_or_default();
            let id = job.id.clone();
            let detail = format!(
                "{} · {} · {} · {}",
                when,
                if job.printer_name.is_empty() { "-" } else { &job.printer_name },
                match job.source {
                    JobSource::Ui => "UI",
                    JobSource::Http => "HTTP",
                    JobSource::Ws => "WS",
                },
                job.config_summary
            );
            list = list.child(
                h_flex()
                    .items_stretch()
                    .rounded_lg()
                    .overflow_hidden()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.group_box)
                    .child(div().w(px(4.)).flex_shrink_0().bg(color))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .px_4()
                            .py_3()
                            .items_center()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(div().text_sm().font_semibold().truncate().child(job.title.clone()))
                                    .child(div().text_xs().text_color(theme.muted_foreground).truncate().child(detail))
                                    .when_some(job.message.clone(), |d, m| d.child(div().text_xs().text_color(theme.danger).child(m))),
                            )
                            .child(tag.child(text))
                            .child(
                                Button::new(("reprint", i))
                                    .small()
                                    .outline()
                                    .label(self.t("重打", "Reprint"))
                                    .on_click(cx.listener(move |this, _, _, _| {
                                        let core = this.core.clone();
                                        let id = id.clone();
                                        this.core.rt.spawn(async move {
                                            match core.job(&id).and_then(|j| j.replay_payload) {
                                                Some(payload) => {
                                                    core.print_html(payload, JobSource::Ui).await;
                                                }
                                                None => core.set_notice(false, "未找到可重打任务"),
                                            }
                                        });
                                    })),
                            ),
                    ),
            );
        }
        let export = Button::new("export")
            .small()
            .label(self.t("导出记录", "Export"))
            .on_click(cx.listener(|this, _, _, cx| {
                let dir = dirs::document_dir().unwrap_or_else(std::env::temp_dir);
                let name = format!("print-jobs-{}.json", chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"));
                let rx = cx.prompt_for_new_path(&dir, Some(&name));
                let core = this.core.clone();
                cx.spawn(async move |_, _| {
                    if let Ok(Ok(Some(path))) = rx.await {
                        match core.export_jobs_to(&path) {
                            Ok(()) => core.set_notice(true, format!("已导出: {}", path.display())),
                            Err(e) => core.set_notice(false, e.to_string()),
                        }
                    }
                })
                .detach();
            }));
        v_flex()
            .gap_5()
            .child(self.page_header(self.t("打印记录", "Jobs"), self.t("最近 50 条任务,可重打或导出", "Last 50 jobs; reprint or export"), export, cx))
            .child(list)
    }
}

fn truncate_status(status: &str) -> String {
    let first: String = status.chars().take(18).collect();
    if status.chars().count() > 18 { format!("{first}…") } else { first }
}

impl Render for ReportApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = match self.tab {
            0 => self.printers_tab(cx).into_any_element(),
            1 => self.config_tab(cx).into_any_element(),
            _ => self.jobs_tab(cx).into_any_element(),
        };
        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.sidebar(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(
                        div()
                            .id("body")
                            .size_full()
                            .overflow_y_scroll()
                            .child(
                                v_flex()
                                    .max_w(px(860.))
                                    .mx_auto()
                                    .p_6()
                                    .child(body),
                            ),
                    )
                    .when_some(self.notice_bar(cx), |d, notice| {
                        d.child(
                            h_flex()
                                .absolute()
                                .bottom_4()
                                .left_4()
                                .right_4()
                                .justify_end()
                                .child(div().max_w(px(400.)).child(notice)),
                        )
                    }),
            )
    }
}
