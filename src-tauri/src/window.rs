//! Where the main window opens — UI-SPEC §5.
//!
//! Until 1.4.0 the window was capped at 1600×1100 and refused to maximize. That answered the wrong
//! problem: the layout reads badly stretched across a 4K panel, and the answer to that is a content
//! column with a maximum width (`max-w-content` in `App.tsx`), not a window the user cannot fill or
//! snap. And the cap never touched the opposite case, which is the one that hid controls:
//! `tauri.conf.json` asks for 1180×760 logical pixels, taller than the work area of a 1920×1080
//! panel at 150 % (about 688 under the taskbar) and of a 1366×768 laptop at 100 % — so the status
//! line and the plan panel's Confirm opened below the taskbar.
//!
//! So the window is created hidden (`visible: false`), fitted here to the work area of the monitor
//! it opened on, centred in it, and only then shown.

/// Logical pixels kept clear between the window and each edge of the work area.
const MARGIN: f64 = 16.0;

/// A rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Where the window goes: its outer position, and the inner size to ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub inner_width: u32,
    pub inner_height: u32,
}

/// Logical to physical, rounded. The inputs are window sizes from a config file and a monitor's
/// scale factor, so the result is small and positive; anything else saturates.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a window dimension times a scale factor; `as` saturates, which is the wanted clamp"
)]
fn physical(logical: f64, scale: f64) -> u32 {
    (logical * scale).round() as u32
}

/// The configured inner size, shrunk to fit `work` but never below the configured minimum, with
/// the outer rectangle centred in `work`.
///
/// `frame` is what the decorations add to the inner size — title bar and borders — in physical
/// pixels; `wanted` and `min` are logical, as `tauri.conf.json` states them. A work area smaller
/// than the minimum gets the minimum, pinned to the work area's top-left rather than centred off
/// its edge, because a title bar above the screen cannot be dragged back.
#[must_use]
pub fn fit(
    work: Rect,
    scale: f64,
    frame: (u32, u32),
    wanted: (f64, f64),
    min: (f64, f64),
) -> Placement {
    let margin = physical(MARGIN, scale);
    let room_w = work.width.saturating_sub(frame.0 + 2 * margin);
    let room_h = work.height.saturating_sub(frame.1 + 2 * margin);
    let inner_width = physical(wanted.0, scale)
        .min(room_w)
        .max(physical(min.0, scale));
    let inner_height = physical(wanted.1, scale)
        .min(room_h)
        .max(physical(min.1, scale));
    let centre = |start: i32, room: u32, outer: u32| {
        start.saturating_add(i32::try_from(room.saturating_sub(outer) / 2).unwrap_or(0))
    };
    Placement {
        x: centre(work.x, work.width, inner_width + frame.0),
        y: centre(work.y, work.height, inner_height + frame.1),
        inner_width,
        inner_height,
    }
}

/// Fit `window` to its monitor's work area, per `config`.
///
/// # Errors
/// Whatever Tauri reports for a window it cannot query or move. The caller shows the window
/// regardless: at the configured size is better than not at all.
pub fn place(
    window: &tauri::WebviewWindow,
    config: &tauri::utils::config::WindowConfig,
) -> tauri::Result<()> {
    let monitor = match window.current_monitor()? {
        Some(monitor) => monitor,
        None => match window.primary_monitor()? {
            Some(monitor) => monitor,
            None => return Ok(()),
        },
    };
    let area = monitor.work_area();
    let outer = window.outer_size()?;
    let inner = window.inner_size()?;
    let placement = fit(
        Rect {
            x: area.position.x,
            y: area.position.y,
            width: area.size.width,
            height: area.size.height,
        },
        monitor.scale_factor(),
        (
            outer.width.saturating_sub(inner.width),
            outer.height.saturating_sub(inner.height),
        ),
        (config.width, config.height),
        (
            config.min_width.unwrap_or(0.0),
            config.min_height.unwrap_or(0.0),
        ),
    );
    window.set_size(tauri::PhysicalSize::new(
        placement.inner_width,
        placement.inner_height,
    ))?;
    window.set_position(tauri::PhysicalPosition::new(placement.x, placement.y))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WANTED: (f64, f64) = (1180.0, 760.0);
    const MIN: (f64, f64) = (900.0, 560.0);
    /// A Windows 11 title bar and resize borders at 100 %.
    const FRAME: (u32, u32) = (16, 39);

    fn work(width: u32, height: u32) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    #[test]
    fn a_large_monitor_gets_the_configured_size_centred() {
        let p = fit(work(1920, 1032), 1.0, FRAME, WANTED, MIN);
        assert_eq!((p.inner_width, p.inner_height), (1180, 760));
        assert_eq!((p.x, p.y), ((1920 - 1196) / 2, (1032 - 799) / 2));
    }

    #[test]
    fn a_scaled_laptop_panel_gets_a_window_that_fits_under_its_taskbar() {
        // 1920×1080 at 150 %: the work area is 1032 physical pixels tall, and 760 logical is 1140.
        let frame = (24, 59);
        let p = fit(work(1920, 1032), 1.5, frame, WANTED, MIN);
        assert_eq!(p.inner_width, 1770, "the width fits, so it is untouched");
        assert!(p.inner_height + frame.1 <= 1032, "{p:?}");
        assert!(p.y >= 0, "{p:?}");
    }

    #[test]
    fn a_small_laptop_keeps_the_whole_window_on_screen() {
        let p = fit(work(1366, 728), 1.0, FRAME, WANTED, MIN);
        assert!(p.inner_height + FRAME.1 <= 728, "{p:?}");
        assert!(p.inner_height >= 560, "never below the minimum");
        assert!(p.y >= 0);
    }

    #[test]
    fn a_work_area_below_the_minimum_gets_the_minimum_at_its_corner() {
        let p = fit(work(800, 600), 1.0, FRAME, WANTED, MIN);
        assert_eq!((p.inner_width, p.inner_height), (900, 560));
        assert_eq!(
            (p.x, p.y),
            (0, 0),
            "never centred off the top of the screen"
        );
    }

    #[test]
    fn a_second_monitor_is_measured_from_its_own_origin() {
        let p = fit(
            Rect {
                x: -1920,
                y: 120,
                width: 1920,
                height: 1040,
            },
            1.0,
            FRAME,
            WANTED,
            MIN,
        );
        assert!(p.x >= -1920 && p.x + 1196 <= 0, "{p:?}");
        assert!(p.y >= 120, "{p:?}");
    }
}
