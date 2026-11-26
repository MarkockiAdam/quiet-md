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

/// The meadow as a soft blur, for behind a sheet that wants the focus (课表's forms): the film's
/// frame averaged down to `BLUR_W` × `BLUR_H` blocks and stretched over the window, where the GPU's
/// smooth sampling melts the blocks into one another. GPUI has no backdrop blur; this is the meadow's
/// own: its pixels, blurred.
const BLUR_W: usize = 40;
const BLUR_H: usize = 25;
/// The wash over the blur, so the sheet stands out and the meadow stays a mood.
const BLUR_WASH_LIGHT: f32 = 0.28;
const BLUR_WASH_DARK: f32 = 0.45;

thread_local! {
    /// The blurred frame, made once: the held frame, so it never moves under a sheet.
    static BLURRED: RefCell<Option<Arc<RenderImage>>> = const { RefCell::new(None) };
}

fn blurred() -> Option<Arc<RenderImage>> {
    if let Some(t) = BLURRED.with(|b| b.borrow().clone()) { return Some(t) }
    let f = film_frames()?;
    let frame = &f.frames[frame_at(f.frames.len(), None)];
    let rgb: Vec<[f32; 3]> = f.colors.iter().map(|&c| { let c = Rgba::from(c); [c.r, c.g, c.b] }).collect();
    let (bw, bh) = (film::FILM_W / BLUR_W, film::FILM_H / BLUR_H);
    let mut buf = Vec::with_capacity(BLUR_W * BLUR_H * 4);
    for by in 0..BLUR_H {
        for bx in 0..BLUR_W {
            let mut sum = [0f32; 3];
            for y in by * bh..(by + 1) * bh {
                for x in bx * bw..(bx + 1) * bw {
                    let c = rgb[(frame[y * film::FILM_W + x] as usize).min(rgb.len() - 1)];
                    for i in 0..3 { sum[i] += c[i]; }
                }
            }
            let n = (bw * bh) as f32;
            // BGRA, as `film::texture` lays it out
            buf.extend_from_slice(&[(sum[2] / n * 255.) as u8, (sum[1] / n * 255.) as u8, (sum[0] / n * 255.) as u8, 255]);
        }
    }
    let img = image::RgbaImage::from_raw(BLUR_W as u32, BLUR_H as u32, buf)?;
    let tex = Arc::new(RenderImage::new(vec![image::Frame::new(img)]));
    BLURRED.with(|b| *b.borrow_mut() = Some(tex.clone()));
    Some(tex)
}

/// How long the blur takes to close in from the edges.
pub const CLOSE_IN: std::time::Duration = std::time::Duration::from_millis(260);

/// Ease out (cubic): quick to start, soft to land.
pub fn ease_out(t: f32) -> f32 { let t = t.clamp(0., 1.); 1. - (1. - t).powi(3) }

/// The blurred meadow and its wash, filling the parent: what a focused sheet sits on. `progress`
/// (0 to 1, already eased) closes it in from all four edges to the middle: a frame whose bands grow
/// until they meet.
pub fn backdrop_at(progress: f32) -> AnyElement {
    let wash = crate::theme::Theme::bg_body().opacity(if crate::theme::dark() { BLUR_WASH_DARK } else { BLUR_WASH_LIGHT });
    let tex = blurred();
    canvas(|b, _, _| b, move |_, b, window, _| {
        let (bw, bh) = (f32::from(b.size.width), f32::from(b.size.height));
        // The bands: `progress` of half the window each way, on whole device pixels and never past the
        // middle, so they meet edge to edge: no seam, no strip where two washes overlap. Once they
        // meet, one layer.
        let sf = window.scale_factor();
        let snap = |v: f32| (v * sf).floor() / sf;
        let (tx, ty) = (snap(bw / 2. * progress).min(snap(bw / 2.)), snap(bh / 2. * progress).min(snap(bh / 2.)));
        let closed = progress >= 1. || bh - 2. * ty <= 1. / sf || bw - 2. * tx <= 1. / sf;
        let bands = if closed { vec![b] } else {
            let (l, t) = (b.left(), b.top());
            let mid = bh - 2. * ty;
            vec![
                Bounds::new(b.origin, size(px(bw), px(ty))),
                Bounds::new(point(l, t + px(bh - ty)), size(px(bw), px(ty))),
                Bounds::new(point(l, t + px(ty)), size(px(tx), px(mid))),
                Bounds::new(point(l + px(bw - tx), t + px(ty)), size(px(tx), px(mid))),
            ].into_iter().filter(|r| r.size.width > px(0.) && r.size.height > px(0.)).collect()
        };
        // scaled to cover, centred, as the film is
        let s = (bw / BLUR_W as f32).max(bh / BLUR_H as f32);
        let (w, h) = (BLUR_W as f32 * s, BLUR_H as f32 * s);
        let ib = Bounds::new(point(b.left() + px((bw - w) / 2.), b.top() + px((bh - h) / 2.)), size(px(w), px(h)));
        for band in bands {
            window.with_content_mask(Some(ContentMask { bounds: band }), |window| {
                if let Some(tex) = &tex { let _ = window.paint_image(ib, ib, Corners::default(), tex.clone(), 0, false); }
                window.paint_quad(fill(band, wash));
            });
        }
    }).absolute().inset_0().into_any_element()
}

/// A focused sheet over the blur, opened `since` ago: the blur closes in from the edges, then the
/// sheet fades in at the middle with a small rise. Asks for frames until it has landed.
pub fn focused(since: web_time::Instant, sheet: AnyElement, window: &mut Window) -> AnyElement {
    focused_at(since.elapsed().as_secs_f32() / CLOSE_IN.as_secs_f32(), sheet, window)
}

/// How long the whole motion runs, in `CLOSE_IN`s: the blur's close, then the sheet landing.
pub const SPAN: f32 = 1.2;

/// The sheet leaving, closed `since` ago: the opening played backwards at the same pace, the sheet
/// fading and sinking, then the blur drawing back from the middle to the edges. None once it is gone.
pub fn leaving(since: web_time::Instant, sheet: AnyElement, window: &mut Window) -> Option<AnyElement> {
    let t = since.elapsed().as_secs_f32() / CLOSE_IN.as_secs_f32();
    if t >= SPAN { return None }
    window.request_animation_frame();
    Some(focused_at(SPAN - t, sheet, window))
}

/// The motion at `t` (in `CLOSE_IN`s, 0 to `SPAN`); asks for frames while it runs.
fn focused_at(t: f32, sheet: AnyElement, window: &mut Window) -> AnyElement {
    if t < SPAN { window.request_animation_frame(); }
    // the sheet arrives with the close's second half, landing just after it
    let s = ease_out(((t - 0.3) / 0.9).clamp(0., 1.));
    div().absolute().inset_0()
        .child(backdrop_at(ease_out(t)))
        .child(div().absolute().inset_0().opacity(s).top(px((1. - s) * 10.)).child(sheet))
        .into_any_element()
}
