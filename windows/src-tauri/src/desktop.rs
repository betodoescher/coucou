// Mochi on the desktop: dragged out of the island, he lives in a small
// transparent window of his own until he is dropped back on the island or
// double-clicked home. Port of DesktopMochi.swift.
//
// The island stays in charge of what Mochi feels and wears and tells this
// window over events; Rust only places the window, flies it, keeps it on
// screen and remembers where it was left.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::island;
use crate::platform::{self, cursor_physical, left_button_down};

pub const LABEL: &str = "mochi";

/// Window size in logical pixels, and where Mochi's body sits in it. Same
/// numbers as src/desktop/main.ts.
pub const WIN_W: f64 = 128.0;
pub const WIN_H: f64 = 144.0;
pub const BODY_X: f64 = 64.0;
pub const BODY_Y: f64 = 90.0;
/// The body takes the mouse; the rest of the window lets it through.
pub const HIT_R: f64 = 36.0;

/// Room kept between Mochi and the edges of the screen's work area.
const EDGE_MARGIN: f64 = 12.0;
/// Dropped this close to the island, he goes home.
const HOME_MARGIN: f64 = 30.0;
const FLIGHT: Duration = Duration::from_millis(450);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DesktopEvent {
    /// The user's choice: Mochi lives on the desktop (kept while he visits the
    /// island for an alert).
    pub on_desktop: bool,
    /// His window is on screen.
    pub visible: bool,
    /// He has just been dropped on the desktop.
    pub landed: bool,
}

#[derive(Serialize, Clone)]
struct CursorPayload {
    x: f64,
    y: f64,
}

static VISIBLE: AtomicBool = AtomicBool::new(false);
/// Bumped by every flight or drag, so an older flight stops moving the window.
static GENERATION: AtomicU64 = AtomicU64::new(0);
static POLL: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// Positioning a window needs a desktop that lets apps do it (not a
/// layer-shell compositor).
pub fn available() -> bool {
    platform::island_movable()
}

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

/// Created hidden at launch, before the island: see create_settings_window.
pub fn create(app: &AppHandle, url: WebviewUrl, browser_args: &str) {
    if !available() {
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, url)
        .additional_browser_args(browser_args)
        .title("Mochi")
        .inner_size(WIN_W, WIN_H)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible_on_all_workspaces(true)
        .maximizable(false)
        .minimizable(false)
        .visible(false)
        .build();
    match built {
        Ok(win) => {
            platform::make_floating(&win);
            let r = HIT_R;
            platform::shape_window(&win, (BODY_X - r, BODY_Y - r, r * 2.0, r * 2.0));
            if platform::CURSOR_POLL {
                spawn_poll(app.clone());
            }
        }
        Err(err) => crate::log::line(format!("desktop Mochi window failed: {err}")),
    }
}

fn scale(win: &WebviewWindow) -> f64 {
    win.scale_factor().unwrap_or(1.0)
}

/// Physical screen point under Mochi's body.
fn body_center(win: &WebviewWindow) -> Option<(f64, f64)> {
    let pos = win.outer_position().ok()?;
    let s = scale(win);
    Some((pos.x as f64 + BODY_X * s, pos.y as f64 + BODY_Y * s))
}

fn place(win: &WebviewWindow, (cx, cy): (f64, f64)) {
    let s = scale(win);
    let x = (cx - BODY_X * s).round() as i32;
    let y = (cy - BODY_Y * s).round() as i32;
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

fn emit(app: &AppHandle, landed: bool) {
    let on_desktop = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().mochi_on_desktop)
        .unwrap_or(false);
    let visible = VISIBLE.load(Ordering::Relaxed);
    let _ = app.emit("desktop-mochi", DesktopEvent { on_desktop, visible, landed });
}

fn show(app: &AppHandle, win: &WebviewWindow) {
    if VISIBLE.swap(true, Ordering::Relaxed) {
        return;
    }
    if platform::CURSOR_POLL {
        // The poll gives the body the mouse back as soon as the cursor is on it.
        let _ = win.set_ignore_cursor_events(true);
    }
    let _ = win.show();
    fit(win);
    let _ = win.set_always_on_top(true);
    let (lock, cv) = &POLL;
    *lock.lock().unwrap() = true;
    cv.notify_all();
    emit(app, false);
}

/// GTK keeps a non-resizable window at its 200 px natural size and tao
/// re-applies `resizable: false` once the window is mapped (see
/// island::apply_geometry), so the size is asked for again once he has landed.
fn fit(win: &WebviewWindow) {
    #[cfg(target_os = "linux")]
    {
        let _ = win.set_resizable(true);
        let _ = win.set_size(tauri::LogicalSize::new(WIN_W, WIN_H));
    }
    let _ = win;
}

fn hide(app: &AppHandle, win: &WebviewWindow) {
    if !VISIBLE.swap(false, Ordering::Relaxed) {
        return;
    }
    let _ = win.hide();
    *POLL.0.lock().unwrap() = false;
    emit(app, false);
}

fn store(app: &AppHandle, change: impl FnOnce(&mut crate::settings::Settings)) {
    let Some(shared) = app.try_state::<crate::Shared>() else { return };
    let updated = {
        let mut current = shared.settings.lock().unwrap();
        change(&mut current);
        current.clone()
    };
    if let Err(err) = crate::settings::save(&updated) {
        crate::log::line(format!("could not save Mochi's spot: {err}"));
    }
    let _ = app.emit("settings-changed", updated);
}

/// Work area (physical x, y, w, h) of the display holding the point, or the
/// primary one.
fn work_area(app: &AppHandle, (x, y): (f64, f64)) -> Option<(f64, f64, f64, f64)> {
    let monitors = app.available_monitors().ok()?;
    let m = monitors
        .iter()
        .find(|m| {
            let p = m.position();
            let s = m.size();
            x >= p.x as f64 && x < p.x as f64 + s.width as f64 && y >= p.y as f64 && y < p.y as f64 + s.height as f64
        })
        .cloned()
        .or_else(|| app.primary_monitor().ok().flatten())
        .or_else(|| monitors.into_iter().next())?;
    let a = m.work_area();
    Some((a.position.x as f64, a.position.y as f64, a.size.width as f64, a.size.height as f64))
}

/// Moves a body centre so the whole window stays inside the work area.
pub fn clamp_center((cx, cy): (f64, f64), (ax, ay, aw, ah): (f64, f64, f64, f64), scale: f64) -> (f64, f64) {
    let m = EDGE_MARGIN * scale;
    let left = ax + m + BODY_X * scale;
    let right = ax + aw - m - (WIN_W - BODY_X) * scale;
    let top = ay + m + BODY_Y * scale;
    let bottom = ay + ah - m - (WIN_H - BODY_Y) * scale;
    (cx.clamp(left, right.max(left)), cy.clamp(top, bottom.max(top)))
}

/// The island shape on screen (physical x, y, w, h), grown by `margin` logical px.
fn island_zone(app: &AppHandle, margin: f64) -> Option<(f64, f64, f64, f64)> {
    let win = island::window(app)?;
    let pos = win.outer_position().ok()?;
    let s = win.scale_factor().unwrap_or(1.0);
    let shared = app.try_state::<crate::Shared>()?;
    let collapsed = shared.gate.collapsed.load(Ordering::Relaxed);
    let r = *shared.gate.rect.lock().unwrap();
    let (x, y, w, h) = if collapsed || r.w <= 0.0 {
        (0.0, 0.0, island::STRIP_W, island::STRIP_H.max(20.0))
    } else {
        (r.x, r.y, r.w, r.h.max(20.0))
    };
    Some((
        pos.x as f64 + (x - margin) * s,
        pos.y as f64 + (y - margin) * s,
        (w + margin * 2.0) * s,
        (h + margin * 2.0) * s,
    ))
}

pub fn in_zone((x, y): (f64, f64), (zx, zy, zw, zh): (f64, f64, f64, f64)) -> bool {
    x >= zx && x <= zx + zw && y >= zy && y <= zy + zh
}

/// Where Mochi sits in the island: the left of its shape, near the top.
fn island_spot(app: &AppHandle) -> Option<(f64, f64)> {
    let (x, y, w, h) = island_zone(app, 0.0)?;
    Some((x + (w / 2.0).min(46.0 * scale_of_island(app)), y + (h / 2.0).min(20.0 * scale_of_island(app))))
}

fn scale_of_island(app: &AppHandle) -> f64 {
    island::window(app).and_then(|w| w.scale_factor().ok()).unwrap_or(1.0)
}

/// The saved spot, put back on whatever screen layout there is now.
fn saved_spot(app: &AppHandle, win: &WebviewWindow) -> Option<(f64, f64)> {
    let spot = app.try_state::<crate::Shared>()?.settings.lock().unwrap().desktop_mochi?;
    let area = work_area(app, spot)?;
    Some(clamp_center(spot, area, scale(win)))
}

/// Physical point to follow during a drag: the real cursor where we can read
/// it, otherwise the page's screen coordinates.
fn drag_point(caller_scale: f64, x: f64, y: f64) -> (f64, f64) {
    cursor_physical().unwrap_or((x * caller_scale, y * caller_scale))
}

/// The pointer grabbed Mochi (in the island or on the desktop). Where Rust can
/// read the mouse it follows it to the drop itself; elsewhere the page keeps
/// sending `drag_to` and ends with `drop`.
pub fn drag_start(app: &AppHandle, caller_scale: f64, x: f64, y: f64) {
    let Some(win) = window(app) else { return };
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    place(&win, drag_point(caller_scale, x, y));
    show(app, &win);
    if !platform::CURSOR_POLL {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        while left_button_down() {
            if GENERATION.load(Ordering::Relaxed) != generation {
                return;
            }
            if let Some(p) = cursor_physical() {
                place(&win, p);
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        drop(&app);
    });
}

pub fn drag_to(app: &AppHandle, caller_scale: f64, x: f64, y: f64) {
    if platform::CURSOR_POLL {
        return;
    }
    let Some(win) = window(app) else { return };
    place(&win, drag_point(caller_scale, x, y));
}

/// End of a drag: on the island he goes home, anywhere else he stays.
pub fn drop(app: &AppHandle) {
    let Some(win) = window(app) else { return };
    let Some(center) = body_center(&win) else { return };
    if island_zone(app, HOME_MARGIN).is_some_and(|zone| in_zone(center, zone)) {
        store(app, |s| s.mochi_on_desktop = false);
        hide(app, &win);
        return;
    }
    let spot = work_area(app, center).map(|a| clamp_center(center, a, scale(&win))).unwrap_or(center);
    crate::log::line(format!("desktop Mochi landed at {:.0},{:.0}", spot.0, spot.1));
    fit(&win);
    place(&win, spot);
    store(app, |s| {
        s.mochi_on_desktop = true;
        s.desktop_mochi = Some(spot);
    });
    emit(app, true);
}

/// Double-click: he flies back into the island for good.
pub fn go_home(app: &AppHandle) {
    store(app, |s| s.mochi_on_desktop = false);
    retract(app);
}

/// The shortcut: home if he is out, otherwise out to where he was last left,
/// or the bottom-right corner the first time.
pub fn toggle(app: &AppHandle) {
    let Some(win) = window(app) else { return };
    let out = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().mochi_on_desktop)
        .unwrap_or(false);
    if out {
        go_home(app);
        return;
    }
    if saved_spot(app, &win).is_none() {
        let Some(from) = island_spot(app) else { return };
        let Some(area) = work_area(app, from) else { return };
        let corner = clamp_center((f64::MAX, f64::MAX), area, scale(&win));
        store(app, |s| s.desktop_mochi = Some(corner));
    }
    store(app, |s| s.mochi_on_desktop = true);
    fly_out(app);
}

/// Back to the island, keeping the user's choice (an alert, or going home).
pub fn retract(app: &AppHandle) {
    let Some(win) = window(app) else { return };
    if !VISIBLE.load(Ordering::Relaxed) {
        emit(app, false);
        return;
    }
    let from = body_center(&win);
    let to = island_spot(app);
    match (from, to) {
        (Some(from), Some(to)) => fly(app, win, from, to, true),
        _ => hide(app, &win),
    }
}

/// From the island to his spot on the desktop (after launch, after an alert).
pub fn fly_out(app: &AppHandle) {
    let Some(win) = window(app) else { return };
    let on_desktop = app
        .try_state::<crate::Shared>()
        .map(|s| s.settings.lock().unwrap().mochi_on_desktop)
        .unwrap_or(false);
    if !on_desktop {
        return;
    }
    let Some(to) = saved_spot(app, &win) else { return };
    let from = island_spot(app).unwrap_or(to);
    crate::log::line(format!("desktop Mochi flies out to {:.0},{:.0}", to.0, to.1));
    place(&win, from);
    show(app, &win);
    fly(app, win, from, to, false);
}

fn fly(app: &AppHandle, win: WebviewWindow, from: (f64, f64), to: (f64, f64), hide_after: bool) {
    let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        loop {
            if GENERATION.load(Ordering::Relaxed) != generation {
                return;
            }
            let t = (start.elapsed().as_secs_f64() / FLIGHT.as_secs_f64()).min(1.0);
            let k = flight_ease(t, hide_after);
            place(&win, (from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k));
            if t >= 1.0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        if hide_after {
            hide(&app, &win);
        } else {
            fit(&win);
            place(&win, to);
        }
    });
}

/// Ease-out leaving the island, ease-in going back into it.
pub fn flight_ease(t: f64, inwards: bool) -> f64 {
    if inwards {
        t * t * t
    } else {
        1.0 - (1.0 - t).powi(3)
    }
}

/// Windows: the body takes the mouse, the rest of the window lets it through,
/// and the page gets the cursor to look at. Parked while Mochi is hidden.
fn spawn_poll(app: AppHandle) {
    std::thread::spawn(move || {
        let mut accepting = false;
        let mut last = (f64::MIN, f64::MIN);
        loop {
            {
                let (lock, cv) = &POLL;
                let mut on = lock.lock().unwrap();
                while !*on {
                    on = cv.wait(on).unwrap();
                    accepting = false;
                }
            }
            std::thread::sleep(Duration::from_millis(33));
            let Some(win) = window(&app) else { continue };
            let Ok(pos) = win.outer_position() else { continue };
            let Some((cx, cy)) = cursor_physical() else { continue };
            let s = scale(&win);
            let (x, y) = ((cx - pos.x as f64) / s, (cy - pos.y as f64) / s);
            let on_body = (x - BODY_X).hypot(y - BODY_Y) <= HIT_R;
            let accept = on_body || (accepting && left_button_down());
            if accept != accepting {
                accepting = accept;
                let _ = win.set_ignore_cursor_events(!accept);
            }
            if (x - last.0).abs() >= 1.0 || (y - last.1).abs() >= 1.0 {
                last = (x, y);
                let _ = win.emit("desktop-cursor", CursorPayload { x, y });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mochi_always_stays_on_screen() {
        let area = (0.0, 0.0, 1920.0, 1040.0);
        assert_eq!(clamp_center((800.0, 500.0), area, 1.0), (800.0, 500.0));
        let (x, y) = clamp_center((-200.0, -200.0), area, 1.0);
        assert_eq!((x, y), (EDGE_MARGIN + BODY_X, EDGE_MARGIN + BODY_Y));
        let (x, y) = clamp_center((5000.0, 5000.0), area, 2.0);
        assert_eq!(x, 1920.0 - EDGE_MARGIN * 2.0 - (WIN_W - BODY_X) * 2.0);
        assert_eq!(y, 1040.0 - EDGE_MARGIN * 2.0 - (WIN_H - BODY_Y) * 2.0);
        // A second display to the left, with negative coordinates.
        let left = (-1280.0, 0.0, 1280.0, 1024.0);
        assert_eq!(clamp_center((-600.0, 400.0), left, 1.0), (-600.0, 400.0));
    }

    #[test]
    fn dropping_near_the_island_sends_mochi_home() {
        let zone = (800.0, 0.0, 320.0, 80.0);
        assert!(in_zone((900.0, 40.0), zone));
        assert!(in_zone((800.0, 80.0), zone));
        assert!(!in_zone((900.0, 81.0), zone));
        assert!(!in_zone((500.0, 40.0), zone));
    }

    #[test]
    fn flights_start_and_end_where_they_should() {
        for inwards in [true, false] {
            assert_eq!(flight_ease(0.0, inwards), 0.0);
            assert_eq!(flight_ease(1.0, inwards), 1.0);
        }
        assert!(flight_ease(0.5, false) > 0.5, "leaving the island starts fast");
        assert!(flight_ease(0.5, true) < 0.5, "going back in starts slow");
    }
}
