//! First-open window size and position limited to a monitor's work area.
//!
//! Default window sizes are logical pixels. On small screens, or with Windows display
//! scaling, the default can be taller than the area above the taskbar. The pure
//! [`fit_to_work_area`] keeps the default when it fits and otherwise shrinks the window
//! (and, if needed, its minimum size) so the whole frame stays inside the work area.

/// A monitor work area in physical pixels, as reported by `Monitor::work_area()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A logical (pre-scaling) window content size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LogicalSize {
    pub width: f64,
    pub height: f64,
}

/// The placement to apply before the window is first shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Logical content size.
    pub inner: LogicalSize,
    /// Logical minimum content size; never larger than `inner`.
    pub min_inner: LogicalSize,
    /// Physical position of the window's outer top-left corner.
    pub x: i32,
    pub y: i32,
}

/// Fits a window into `work_area` and centers it there.
///
/// `frame_width` and `frame_height` are the physical pixels the title bar and borders add
/// around the content (outer size minus inner size). `scale_factor` converts logical to
/// physical pixels for the monitor that holds the work area.
pub fn fit_to_work_area(
    work_area: WorkArea,
    scale_factor: f64,
    frame_width: u32,
    frame_height: u32,
    default_inner: LogicalSize,
    min_inner: LogicalSize,
) -> Placement {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let available = |work: u32, frame: u32| f64::from(work.saturating_sub(frame)) / scale;
    // Floor so the rounded physical size never exceeds the work area.
    let fit = |wanted: f64, limit: f64| wanted.min(limit.floor()).max(1.0);
    let inner = LogicalSize {
        width: fit(default_inner.width, available(work_area.width, frame_width)),
        height: fit(
            default_inner.height,
            available(work_area.height, frame_height),
        ),
    };
    let min_inner = LogicalSize {
        width: min_inner.width.min(inner.width),
        height: min_inner.height.min(inner.height),
    };
    let outer_width = (inner.width * scale).round() + f64::from(frame_width);
    let outer_height = (inner.height * scale).round() + f64::from(frame_height);
    let center = |origin: i32, work: u32, outer: f64| {
        let offset = ((f64::from(work) - outer) / 2.0).floor().max(0.0);
        origin.saturating_add(offset as i32)
    };
    Placement {
        inner,
        min_inner,
        x: center(work_area.x, work_area.width, outer_width),
        y: center(work_area.y, work_area.height, outer_height),
    }
}

/// Applies [`fit_to_work_area`] to a hidden, newly built window.
///
/// Uses the monitor the window was created on, or the primary monitor. When no monitor
/// can be read the window keeps its configured size, so a missing monitor never blocks
/// startup.
#[cfg(windows)]
pub fn fit_window(
    window: &tauri::WebviewWindow,
    default_inner: LogicalSize,
    min_inner: LogicalSize,
) -> Result<(), String> {
    let monitor = match window
        .current_monitor()
        .map_err(|error| error.to_string())?
    {
        Some(monitor) => Some(monitor),
        None => window
            .primary_monitor()
            .map_err(|error| error.to_string())?,
    };
    let Some(monitor) = monitor else {
        return Ok(());
    };
    let outer = window.outer_size().map_err(|error| error.to_string())?;
    let inner = window.inner_size().map_err(|error| error.to_string())?;
    let rect = monitor.work_area();
    let placement = fit_to_work_area(
        WorkArea {
            x: rect.position.x,
            y: rect.position.y,
            width: rect.size.width,
            height: rect.size.height,
        },
        monitor.scale_factor(),
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
        default_inner,
        min_inner,
    );
    let logical = |size: LogicalSize| tauri::LogicalSize::new(size.width, size.height);
    window
        .set_min_size(Some(logical(placement.min_inner)))
        .map_err(|error| error.to_string())?;
    window
        .set_size(logical(placement.inner))
        .map_err(|error| error.to_string())?;
    window
        .set_position(tauri::PhysicalPosition::new(placement.x, placement.y))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: LogicalSize = LogicalSize {
        width: 1380.0,
        height: 860.0,
    };
    const MAIN_MIN: LogicalSize = LogicalSize {
        width: 980.0,
        height: 640.0,
    };
    const CONSOLE: LogicalSize = LogicalSize {
        width: 1020.0,
        height: 820.0,
    };
    const CONSOLE_MIN: LogicalSize = LogicalSize {
        width: 460.0,
        height: 280.0,
    };

    fn outer(placement: &Placement, scale: f64, frame: (u32, u32)) -> (f64, f64) {
        (
            (placement.inner.width * scale).round() + f64::from(frame.0),
            (placement.inner.height * scale).round() + f64::from(frame.1),
        )
    }

    fn assert_inside(placement: &Placement, area: WorkArea, scale: f64, frame: (u32, u32)) {
        let (width, height) = outer(placement, scale, frame);
        assert!(
            placement.x >= area.x && placement.y >= area.y,
            "{placement:?}"
        );
        assert!(
            f64::from(placement.x - area.x) + width <= f64::from(area.width),
            "{placement:?}"
        );
        assert!(
            f64::from(placement.y - area.y) + height <= f64::from(area.height),
            "{placement:?}"
        );
        assert!(placement.min_inner.width <= placement.inner.width);
        assert!(placement.min_inner.height <= placement.inner.height);
    }

    #[test]
    fn large_work_area_keeps_default_size_and_centers() {
        let area = WorkArea {
            x: 0,
            y: 0,
            width: 2560,
            height: 1392,
        };
        let placement = fit_to_work_area(area, 1.0, 16, 39, MAIN, MAIN_MIN);
        assert_eq!(placement.inner, MAIN);
        assert_eq!(placement.min_inner, MAIN_MIN);
        assert_eq!(
            (placement.x, placement.y),
            ((2560 - 1396) / 2, (1392 - 899) / 2)
        );
    }

    #[test]
    fn screen_1366x768_at_100_percent_shrinks_height() {
        // 768 minus a 48 px taskbar.
        let area = WorkArea {
            x: 0,
            y: 0,
            width: 1366,
            height: 720,
        };
        let frame = (16, 39);
        let main = fit_to_work_area(area, 1.0, frame.0, frame.1, MAIN, MAIN_MIN);
        assert_eq!(
            main.inner,
            LogicalSize {
                width: 1350.0,
                height: 681.0
            }
        );
        assert_eq!(main.min_inner, MAIN_MIN);
        assert_eq!((main.x, main.y), (0, 0));
        assert_inside(&main, area, 1.0, frame);

        let console = fit_to_work_area(area, 1.0, frame.0, frame.1, CONSOLE, CONSOLE_MIN);
        assert_eq!(
            console.inner,
            LogicalSize {
                width: 1020.0,
                height: 681.0
            }
        );
        assert_inside(&console, area, 1.0, frame);
    }

    #[test]
    fn screen_1920x1080_at_125_percent_uses_logical_pixels() {
        // 1080 minus a 60 px (48 logical) taskbar; frame is 20 x 49 physical.
        let area = WorkArea {
            x: 0,
            y: 0,
            width: 1920,
            height: 1020,
        };
        let frame = (20, 49);
        let main = fit_to_work_area(area, 1.25, frame.0, frame.1, MAIN, MAIN_MIN);
        assert_eq!(
            main.inner,
            LogicalSize {
                width: 1380.0,
                height: 776.0
            }
        );
        assert_inside(&main, area, 1.25, frame);

        let console = fit_to_work_area(area, 1.25, frame.0, frame.1, CONSOLE, CONSOLE_MIN);
        assert_eq!(
            console.inner,
            LogicalSize {
                width: 1020.0,
                height: 776.0
            }
        );
        assert_inside(&console, area, 1.25, frame);
    }

    #[test]
    fn work_area_smaller_than_minimum_lowers_minimum_too() {
        let area = WorkArea {
            x: 0,
            y: 0,
            width: 800,
            height: 560,
        };
        let frame = (16, 39);
        let placement = fit_to_work_area(area, 1.0, frame.0, frame.1, MAIN, MAIN_MIN);
        assert_eq!(
            placement.inner,
            LogicalSize {
                width: 784.0,
                height: 521.0
            }
        );
        assert_eq!(placement.min_inner, placement.inner);
        assert_inside(&placement, area, 1.0, frame);
    }

    #[test]
    fn secondary_monitor_offset_is_kept() {
        let area = WorkArea {
            x: -1920,
            y: 200,
            width: 1920,
            height: 1040,
        };
        let placement = fit_to_work_area(area, 1.0, 16, 39, MAIN, MAIN_MIN);
        assert_eq!(placement.inner, MAIN);
        assert_eq!((placement.x, placement.y), (-1920 + 262, 200 + 70));
        assert_inside(&placement, area, 1.0, (16, 39));
    }

    #[test]
    fn invalid_scale_factor_falls_back_to_one() {
        let area = WorkArea {
            x: 0,
            y: 0,
            width: 1366,
            height: 720,
        };
        let placement = fit_to_work_area(area, 0.0, 16, 39, MAIN, MAIN_MIN);
        assert_eq!(placement.inner.height, 681.0);
    }
}
