//! System tray (macOS menu bar / Windows notification area). Linux has no tray.

#[cfg(not(target_os = "linux"))]
mod imp {
    use std::cell::Cell;
    #[cfg(windows)]
    use std::time::Duration;
    use std::time::Instant;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    pub enum TrayAction {
        Show,
        Quit,
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub struct Tray {
        _icon: TrayIcon,
        show_id: tray_icon::menu::MenuId,
        quit_id: tray_icon::menu::MenuId,
        dark_glyph: Cell<bool>,
        last_theme_check: Cell<Instant>,
    }

    /// Whether the Windows taskbar uses the light theme.
    #[cfg(windows)]
    fn taskbar_is_light() -> bool {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "SystemUsesLightTheme",
            ])
            .creation_flags(0x0800_0000)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim_end().ends_with("0x1"))
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    fn taskbar_is_light() -> bool {
        false
    }

    fn load_icon(dark_glyph: bool) -> Option<Icon> {
        // macOS template icons must be black; the system tints them for light/dark bars.
        let color = if cfg!(target_os = "macos") || dark_glyph { [0, 0, 0] } else { [255, 255, 255] };
        let (w, h, rgba) = crate::brand::tray_rgba(color)?;
        Icon::from_rgba(rgba, w, h).ok()
    }

    impl Tray {
        /// Must be created on the main thread.
        pub fn new() -> Option<Self> {
            let menu = Menu::new();
            let show = MenuItem::new("显示窗口 / Show", true, None);
            let quit = MenuItem::new("退出 / Quit", true, None);
            menu.append_items(&[&show, &quit]).ok()?;
            let dark = taskbar_is_light();
            let builder = TrayIconBuilder::new()
                .with_tooltip("Report Printer Client")
                .with_icon(load_icon(dark)?)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false);
            #[cfg(target_os = "macos")]
            let builder = builder.with_icon_templated(load_icon(dark)?);
            let icon = builder.build().ok()?;
            Some(Self {
                _icon: icon,
                show_id: show.id().clone(),
                quit_id: quit.id().clone(),
                dark_glyph: Cell::new(dark),
                last_theme_check: Cell::new(Instant::now()),
            })
        }

        /// Re-tint the icon when the Windows taskbar theme changes.
        #[cfg(windows)]
        fn sync_theme(&self) {
            if self.last_theme_check.get().elapsed() < Duration::from_secs(5) {
                return;
            }
            self.last_theme_check.set(Instant::now());
            let dark = taskbar_is_light();
            if dark != self.dark_glyph.get() {
                self.dark_glyph.set(dark);
                let _ = self._icon.set_icon(load_icon(dark));
            }
        }

        pub fn poll(&self) -> Option<TrayAction> {
            #[cfg(windows)]
            self.sync_theme();
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == self.quit_id {
                    return Some(TrayAction::Quit);
                }
                if event.id == self.show_id {
                    return Some(TrayAction::Show);
                }
            }
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                    return Some(TrayAction::Show);
                }
            }
            None
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub enum TrayAction {
        Show,
        Quit,
    }

    pub struct Tray;

    impl Tray {
        pub fn new() -> Option<Self> {
            None
        }

        pub fn poll(&self) -> Option<TrayAction> {
            None
        }
    }
}

pub use imp::{Tray, TrayAction};
