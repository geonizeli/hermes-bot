//! `Theme`: the Omarchy palette and Hyprland rounding, as a QML singleton
//! that updates live.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, accent)]
        #[qproperty(QString, accent_foreground, cxx_name = "accentForeground")]
        #[qproperty(QString, background)]
        #[qproperty(QString, surface)]
        #[qproperty(QString, raised)]
        #[qproperty(QString, foreground)]
        #[qproperty(QString, muted)]
        #[qproperty(QString, border)]
        #[qproperty(QString, danger)]
        #[qproperty(i32, radius)]
        type Theme = super::ThemeRust;
    }

    impl cxx_qt::Threading for Theme {}
    impl cxx_qt::Initialize for Theme {}
}

use core::pin::Pin;
use cxx_qt::Threading;
use cxx_qt_lib::QString;

use crate::backend::theme::{self, Palette};

pub struct ThemeRust {
    accent: QString,
    accent_foreground: QString,
    background: QString,
    surface: QString,
    raised: QString,
    foreground: QString,
    muted: QString,
    border: QString,
    danger: QString,
    radius: i32,
}

impl Default for ThemeRust {
    fn default() -> Self {
        let p = Palette::default();
        ThemeRust {
            accent_foreground: QString::from(theme::readable_on(&p.accent)),
            accent: QString::from(&p.accent),
            background: QString::from(&p.background),
            surface: QString::from(&p.surface),
            raised: QString::from(&p.raised),
            foreground: QString::from(&p.foreground),
            muted: QString::from(&p.muted),
            border: QString::from(&p.border),
            danger: QString::from(&p.danger),
            radius: 0,
        }
    }
}

impl qobject::Theme {
    fn apply_palette(mut self: Pin<&mut Self>, p: &Palette) {
        // Setters only emit when the value really changes.
        self.as_mut().set_accent_foreground(QString::from(theme::readable_on(&p.accent)));
        self.as_mut().set_accent(QString::from(&p.accent));
        self.as_mut().set_background(QString::from(&p.background));
        self.as_mut().set_surface(QString::from(&p.surface));
        self.as_mut().set_raised(QString::from(&p.raised));
        self.as_mut().set_foreground(QString::from(&p.foreground));
        self.as_mut().set_muted(QString::from(&p.muted));
        self.as_mut().set_border(QString::from(&p.border));
        self.as_mut().set_danger(QString::from(&p.danger));
    }
}

impl cxx_qt::Initialize for qobject::Theme {
    fn initialize(mut self: Pin<&mut Self>) {
        let dir = theme::omarchy_current_dir();
        let palette = theme::read_palette(&dir);
        self.as_mut().apply_palette(&palette);
        // Corners stay sharp outside Hyprland, like Omarchy's default.
        if let Some(radius) = theme::hyprland_rounding() {
            self.as_mut().set_radius(radius);
        }

        let qt = self.qt_thread();
        let palette_dir = dir.clone();
        theme::watch_palette(dir, move || {
            let palette = theme::read_palette(&palette_dir);
            let _ = qt.queue(move |theme| theme.apply_palette(&palette));
        });

        let qt = self.qt_thread();
        theme::watch_hyprland_config(move || {
            if let Some(radius) = theme::hyprland_rounding() {
                let _ = qt.queue(move |theme| theme.set_radius(radius));
            }
        });
    }
}
