//! 壁纸 (fork feature): the first launch's pixel-art meadow behind the window, so the sidebar sits on
//! it. The page is opaque and covers it. 外观 › 壁纸: "" plays the film at its own 12 fps while
//! the window is active (one held frame when it is not), "still" holds one frame, "off" none.
//! A wash over the picture keeps the sidebar's text legible on the bright sky and leaves.

use crate::onboarding::film::{self, Film};
use gpui::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use web_time::Instant;

/// `Preferences::wallpaper`'s values, in the order 外观 › 壁纸 shows them.
pub const MODES: [&str; 3] = ["", "still", "off"];

/// The frame a held picture shows: the middle of the loop, branches mid-sway.
const STILL: f32 = 0.5;

/// The wash over the picture: the window's own tint, at this share.
const WASH_LIGHT: f32 = 0.42;
const WASH_DARK: f32 = 0.62;

static FILM: OnceLock<Option<Film>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

thread_local! {
    /// Each window's current frame on the GPU, by its index in the loop.
    static SHOWN: RefCell<HashMap<WindowId, (usize, Arc<RenderImage>)>> = RefCell::new(HashMap::new());
}

fn mode() -> String { crate::settings::prefs().wallpaper.clone() }
pub fn on() -> bool { mode() != "off" }
fn animated() -> bool { mode().is_empty() }

fn film_frames() -> Option<&'static Film> { FILM.get_or_init(film::decode).as_ref() }

/// Which frame of the back-and-forth loop plays at `t` seconds; a share of it when held.
fn frame_at(n: usize, t: Option<f32>) -> usize {
    let Some(t) = t else { return ((n - 1) as f32 * STILL) as usize };
    let k = (t * film::FILM_FPS) as usize % (2 * n - 2).max(1);
    if k < n { k } else { 2 * n - 2 - k }
}

/// The picture and its wash, to lay under everything else in the window; None when 壁纸 is off.
pub fn layer(window: &mut Window) -> Option<AnyElement> {
    let id = window.window_handle().window_id();
    if !on() {
        if let Some((_, old)) = SHOWN.with(|s| s.borrow_mut().remove(&id)) { let _ = window.drop_image(old); }
        return None;
    }
    let f = film_frames()?;
    let t = (animated() && window.is_window_active()).then(|| START.get_or_init(Instant::now).elapsed().as_secs_f32());
    // An inactive window keeps the frame it had rather than jumping to the held one.
    let held = SHOWN.with(|s| s.borrow().get(&id).map(|(k, _)| *k));
    let k = if t.is_none() && animated() { held.unwrap_or_else(|| frame_at(f.frames.len(), None)) } else { frame_at(f.frames.len(), t) };
    let tex = match SHOWN.with(|s| s.borrow().get(&id).filter(|(i, _)| *i == k).map(|(_, t)| t.clone())) {
        Some(t) => t,
        None => {
            let tex = film::texture(&f.frames[k], &f.colors);
            if let Some((_, old)) = SHOWN.with(|s| s.borrow_mut().insert(id, (k, tex.clone()))) { let _ = window.drop_image(old); }
            tex
        }
    };
    let wash = crate::theme::Theme::window().opacity(1.).opacity(if crate::theme::dark() { WASH_DARK } else { WASH_LIGHT });
    Some(div().absolute().inset_0()
        .child(canvas(|b, _, _| b, move |_, b, window, _| film::paint_texture(tex, b, window)).absolute().inset_0())
        .child(div().absolute().inset_0().bg(wash))
        .into_any_element())
}

/// Moves the picture on, a frame at a time, while the film plays and the window is in front.
/// Ends with the window.
pub fn tick<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    cx.spawn_in(window, async move |this, cx| {
        // Waking 12 times a second costs a few percent of a core even with nothing to draw: when the
        // film is not playing (window behind, 静止, 关闭) look again only once a second.
        let (frame, idle) = (Duration::from_secs_f32(1. / film::FILM_FPS), Duration::from_secs(1));
        let mut wait = idle;
        loop {
            cx.background_executor().timer(wait).await;
            let Ok(playing) = this.update_in(cx, |_, window, cx| {
                let playing = animated() && window.is_window_active();
                if playing { cx.notify(); }
                playing
            }) else { break };
            wait = if playing { frame } else { idle };
        }
    }).detach();
}
