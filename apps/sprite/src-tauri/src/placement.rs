//! Putting the sprite in the bottom-right corner, above everything, without taking focus, with
//! room for her bubble and input box.
//!
//! On Wayland compositors with the layer-shell protocol (Hyprland, Sway, KDE, …) she goes on
//! the overlay layer, which needs no window rules. That surface has a fixed size with room for
//! a bubble (WebKit doesn't repaint a layer surface that grows), and only her visible parts take
//! clicks. Everywhere else (X11, macOS, Windows, GNOME) she's a small always-on-top window that
//! grows when the bubble or the input box needs room.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use tauri::{AppHandle, LogicalSize, Manager, PhysicalPosition, WebviewWindow};

/// Gap between the sprite and the screen's edges, in logical pixels.
const MARGIN: f64 = 24.0;
/// Room for the bubble and input box on a layer surface: at least this wide, and this much
/// taller than the sprite.
#[cfg(target_os = "linux")]
const PANEL_WIDTH: f64 = 340.0;
#[cfg(target_os = "linux")]
const PANEL_HEIGHT: f64 = 440.0;

/// Whether she's shown (`lucy sprite toggle` hides her).
static VISIBLE: AtomicBool = AtomicBool::new(true);

pub fn place(window: &WebviewWindow, (width, height): (f64, f64)) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    if layer_shell::place(window, (width, height))? {
        return Ok(());
    }
    window.set_always_on_top(true)?;
    resize_window(window, width, height)?;
    window.show()
}

/// Size the window and keep it in the work area's bottom-right corner.
fn resize_window(window: &WebviewWindow, width: f64, height: f64) -> tauri::Result<()> {
    window.set_size(LogicalSize::new(width, height))?;
    if let Some(monitor) = window.primary_monitor()? {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let x = area.position.x + area.size.width as i32 - ((width + MARGIN) * scale) as i32;
        let y = area.position.y + area.size.height as i32 - ((height + MARGIN) * scale) as i32;
        // Wayland without layer shell ignores this; the compositor decides.
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    Ok(())
}

/// A part of the page that takes clicks, in logical pixels, placed by its distance from the
/// window's bottom-right corner (where everything hangs), so it holds at any window size.
#[derive(Debug, Clone, Copy, Deserialize)]
// Only layer shell (Linux) reads them; elsewhere the window simply resizes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct Region {
    pub right: f64,
    pub bottom: f64,
    pub width: f64,
    pub height: f64,
}

/// The page's layout changed: the size it needs, the parts that take clicks, whether the
/// input box wants the keyboard, and whether a bubble is up (Esc hides it). (Not async, so
/// Tauri runs it on the main thread, where GTK must be used.)
#[tauri::command]
pub fn set_layout(
    window: WebviewWindow,
    width: f64,
    height: f64,
    regions: Vec<Region>,
    typing: bool,
    bubble: bool,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if layer_shell::set_layout(&regions, typing, bubble) {
        return Ok(());
    }
    // Only layer shell (Linux) uses these.
    let _ = (regions, bubble);
    resize_window(&window, width, height).map_err(|err| err.to_string())?;
    if typing {
        window.set_focus().map_err(|err| err.to_string())?;
    }
    Ok(())
}

/// Show her if hidden.
pub fn show(app: &AppHandle) {
    set_visible(app, true);
}

/// Hide her if shown, show her if hidden.
pub fn toggle(app: &AppHandle) {
    set_visible(app, !VISIBLE.load(Ordering::SeqCst));
}

fn set_visible(app: &AppHandle, visible: bool) {
    VISIBLE.store(visible, Ordering::SeqCst);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        #[cfg(target_os = "linux")]
        if layer_shell::set_visible(visible) {
            return;
        }
        if let Some(window) = handle.get_webview_window("sprite") {
            let _ = if visible {
                window.show()
            } else {
                window.hide()
            };
        }
    });
}

#[cfg(target_os = "linux")]
mod layer_shell {
    use std::cell::RefCell;

    use gtk::prelude::*;
    use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
    use tauri::WebviewWindow;

    use super::Region;

    thread_local! {
        /// The layer-shell window, when she's on one (GTK objects stay on the main thread).
        static LAYER: RefCell<Option<(gtk::Window, (i32, i32))>> = const { RefCell::new(None) };
    }

    /// Move the webview onto the overlay layer. False when the compositor has no layer shell
    /// (or isn't Wayland), so the caller falls back to a normal window.
    pub fn place(window: &WebviewWindow, (width, height): (f64, f64)) -> tauri::Result<bool> {
        if !gtk_layer_shell::is_supported()
            || std::env::var_os("LUCY_SPRITE_NO_LAYER_SHELL").is_some()
        {
            return Ok(false);
        }
        // A layer surface must be set up before GTK realizes its window, and Tauri's window is
        // realized already: move the webview's container into a fresh window instead. Tauri's
        // own window stays hidden.
        let tauri_window = window.gtk_window()?;
        let content = window.default_vbox()?;
        tauri_window.remove(&content);

        let layer = gtk::Window::new(gtk::WindowType::Toplevel);
        layer.init_layer_shell();
        layer.set_namespace("lucy-sprite");
        layer.set_layer(Layer::Overlay);
        layer.set_anchor(Edge::Bottom, true);
        layer.set_anchor(Edge::Right, true);
        layer.set_layer_shell_margin(Edge::Bottom, super::MARGIN as i32);
        layer.set_layer_shell_margin(Edge::Right, super::MARGIN as i32);
        layer.set_keyboard_mode(KeyboardMode::None);
        layer.set_app_paintable(true);
        if let Some(visual) = WidgetExt::screen(&layer).and_then(|screen| screen.rgba_visual()) {
            layer.set_visual(Some(&visual));
        }
        let size = (
            width.max(super::PANEL_WIDTH).round() as i32,
            (height + super::PANEL_HEIGHT).round() as i32,
        );
        layer.set_size_request(size.0, size.1);
        layer.add(&content);
        layer.show_all();
        LAYER.with(|cell| *cell.borrow_mut() = Some((layer, size)));
        // Until the page says otherwise, only her figure takes clicks.
        let sprite = Region {
            right: 0.0,
            bottom: 0.0,
            width,
            height,
        };
        set_layout(&[sprite], false, false);
        Ok(true)
    }

    /// Let clicks through outside `regions`, and take the keyboard while typing (or offer to,
    /// while a bubble is up). False when she isn't on a layer.
    pub fn set_layout(regions: &[Region], typing: bool, bubble: bool) -> bool {
        LAYER.with(|cell| {
            let Some((layer, (width, height))) = cell.borrow().clone() else {
                return false;
            };
            if let Some(surface) = layer.window() {
                let rectangles: Vec<gtk::cairo::RectangleInt> = regions
                    .iter()
                    .map(|region| {
                        let (w, h) = (region.width.ceil() as i32, region.height.ceil() as i32);
                        gtk::cairo::RectangleInt::new(
                            width - region.right.floor() as i32 - w,
                            height - region.bottom.floor() as i32 - h,
                            w,
                            h,
                        )
                    })
                    .collect();
                let region = gtk::cairo::Region::create_rectangles(&rectangles);
                surface.input_shape_combine_region(&region, 0, 0);
            }
            // Exclusive while typing: a hotkey-opened box gets the keys without a click. On
            // demand while a bubble is up: she keeps focus after you send (so Esc hides her
            // reply) and gets it back when clicked, but gives it up when you click elsewhere.
            layer.set_keyboard_mode(if typing {
                KeyboardMode::Exclusive
            } else if bubble {
                KeyboardMode::OnDemand
            } else {
                KeyboardMode::None
            });
            true
        })
    }

    /// Show or hide the layer surface. False when she isn't on a layer.
    pub fn set_visible(visible: bool) -> bool {
        LAYER.with(|cell| {
            let Some((layer, _)) = cell.borrow().clone() else {
                return false;
            };
            if visible {
                layer.show_all();
            } else {
                layer.hide();
            }
            true
        })
    }
}
