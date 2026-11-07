//! Putting the sprite in the bottom-right corner, above everything, without taking focus.
//!
//! On Wayland compositors with the layer-shell protocol (Hyprland, Sway, KDE, …) she goes on
//! the overlay layer, which needs no window rules. Everywhere else (X11, macOS, Windows,
//! GNOME) she's a small always-on-top window, positioned where the platform allows it.

use tauri::{LogicalSize, PhysicalPosition, WebviewWindow};

/// Gap between the sprite and the screen's edges, in logical pixels.
const MARGIN: f64 = 24.0;

pub fn place(window: &WebviewWindow, (width, height): (f64, f64)) -> tauri::Result<()> {
    #[cfg(target_os = "linux")]
    if layer_shell::place(window, (width, height))? {
        return Ok(());
    }

    window.set_size(LogicalSize::new(width, height))?;
    window.set_always_on_top(true)?;
    if let Some(monitor) = window.primary_monitor()? {
        let scale = monitor.scale_factor();
        let area = monitor.work_area();
        let x = area.position.x + area.size.width as i32 - ((width + MARGIN) * scale) as i32;
        let y = area.position.y + area.size.height as i32 - ((height + MARGIN) * scale) as i32;
        // Wayland without layer shell ignores this; the compositor decides.
        window.set_position(PhysicalPosition::new(x, y))?;
    }
    window.show()
}

#[cfg(target_os = "linux")]
mod layer_shell {
    use gtk::prelude::*;
    use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
    use tauri::WebviewWindow;

    /// Move the webview onto the overlay layer. False when the compositor has no layer shell
    /// (or isn't Wayland), so the caller falls back to a normal window.
    pub fn place(window: &WebviewWindow, (width, height): (f64, f64)) -> tauri::Result<bool> {
        if !gtk_layer_shell::is_supported() {
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
        layer.set_size_request(width.round() as i32, height.round() as i32);
        layer.add(&content);
        layer.show_all();
        Ok(true)
    }
}
