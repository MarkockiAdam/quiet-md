//! 课表 (fork feature): a student's calendar. Classes repeat weekly; assignments, exams and events
//! sit on one day. Laid out like a desk calendar: a large month title, a day or week of all 24
//! hours that opens at the time now, a red line and time pill for now, clashing blocks side by
//! side, deadlines in an all-day row; beside it a month at a glance, what is up next, today and
//! what is due. The sidebar's card says the same in two lines. The data lives in `model.rs`; this
//! is its page, its own entity so the workspace only hosts it.

pub mod calendars;
mod external;
pub mod links;
pub mod model;

use crate::design::{self, Button, ButtonSize, Checkbox, Field, Label, MenuItem, Popover, Segment, Segmented, color};
use crate::input::{TextField, TextFieldEvent};
use gpui::{prelude::FluentBuilder, *};
use model::{Class, Day, Item, Kind, Link, Minute};
use std::collections::HashMap;
use std::time::Duration;

/// What the page asks of the workspace: notes live there.
pub enum ScheduleEvent {
    /// Open this note.
    Open(String),
    /// A new note for `link`, titled `title`, in the folder named after `course` if there is one.
    NewNote { link: Link, title: String, course: String },
    /// Link `note` to `link` (None: unlink it).
    Link { note: String, link: Option<Link> },
}
impl EventEmitter<ScheduleEvent> for SchedulePage {}

/// One hour of the grid.
const HOUR_H: f32 = 52.;
/// The margin with the hour labels.
const GUTTER_W: f32 = 60.;
const PANEL_W: f32 = 296.;
/// Below this window width the side panel steps aside for the grid.
const PANEL_MIN_WINDOW: f32 = 1080.;
/// How far back an unfinished item still shows under 待完成 (overdue).
const OVERDUE_DAYS: i64 = 14;
/// A timed exam or event without an end takes this long in the grid.
const DEFAULT_LEN: Minute = 60;
/// Opening, the grid shows the time now this far below its top.
const LEAD_HOURS: f32 = 1.5;

#[derive(Clone, PartialEq)]
enum Form {
    Class { id: Option<String>, day: u8, color: String },
    Item { id: Option<String>, kind: Kind, done: bool },
}

pub struct SchedulePage {
    pub data: model::Schedule,
    /// A day in the range shown; the range is its week (or the day alone).
    anchor: Day,
    /// 7: the week, 1: the day.
    days: u8,
    /// The month the little calendar shows when moved off the anchor's: (year, month).
    mini: Option<(i64, u32)>,
    add_menu: bool,
    form: Option<Form>,
    error: Option<&'static str>,
    scroll: ScrollHandle,
    /// Notes linked to the schedule, by `Link::key`: (note id, title). The workspace fills it.
    pub links: HashMap<String, Vec<(String, String)>>,
    /// Every note, most recently opened first: (id, title), for 关联笔记. The workspace fills it.
    pub notes: Vec<(String, String)>,
    /// The details card open on this lecture or item.
    detail: Option<Link>,
    /// The card's 关联已有笔记 list is open.
    choosing: bool,
    /// 关联到课表 open for this note (from the note's header).
    picking: Option<String>,
    find: Entity<TextField>,
    /// Google, iCloud …: the calendars of this Mac (`external`).
    ext: external::Ext,
    /// The grid was brought to the time now (on opening, and on 今天).
    placed: bool,
    name: Entity<TextField>,
    kind: Entity<TextField>,
    place: Entity<TextField>,
    start: Entity<TextField>,
    end: Entity<TextField>,
    course: Entity<TextField>,
    date: Entity<TextField>,
    time: Entity<TextField>,
    until: Entity<TextField>,
    _subs: Vec<Subscription>,
}

/// Now, as the schedule counts it: (day, minute, weekday).
fn now() -> (Day, Minute, u8) { model::local(crate::time::now()) }

/// A course colour's ink (text, edges) and its wash (block fill).
fn ink(c: &str) -> Hsla { crate::theme::text_color(c).unwrap_or_else(color::muted_foreground) }
fn wash(c: &str) -> Hsla { ink(c).opacity(if crate::theme::dark() { 0.26 } else { 0.13 }) }
/// The red of now and of today (the design's destructive, close to the system red).
fn now_red() -> Hsla { color::destructive() }

/// One thing drawn in the grid.
struct Block { link: Link, start: Minute, end: Minute, title: String, sub: String, accent: Hsla, fill: Hsla, done: bool }

impl SchedulePage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let field = |cx: &mut Context<Self>, hint: &'static str| cx.new(|cx| TextField::new(hint, cx));
        let (name, kind, place, start, end) = (field(cx, "Calculus"), field(cx, "Lecture"), field(cx, "Room B12"), field(cx, "9:00"), field(cx, "10:30"));
        let (course, date, time, until) = (field(cx, "Calculus"), field(cx, "fri, 3.12, 2026-12-03"), field(cx, "optional"), field(cx, "optional"));
        let find = field(cx, "Search notes");
        let mut subs = vec![];
        subs.push(cx.subscribe(&find, |_: &mut Self, _, _: &TextFieldEvent, cx| cx.notify()));
        for f in [&name, &kind, &place, &start, &end, &course, &date, &time, &until] {
            subs.push(cx.subscribe(f, |p: &mut Self, _, ev: &TextFieldEvent, cx| match ev {
                TextFieldEvent::Submit if p.ext.form.is_some() => p.save_ext_form(cx),
                TextFieldEvent::Submit if p.form.is_none() => {}
                TextFieldEvent::Submit => p.save_form(cx),
                TextFieldEvent::Changed => { p.error = None; cx.notify(); }
                #[allow(unreachable_patterns)]
                _ => {}
            }));
        }
        // The time moves on: the now line, 正在上课 and the countdowns follow it every half minute.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(30)).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() { break }
        }).detach();
        log::info!("calendars: access at start {:?}", calendars::access());
        SchedulePage {
            data: model::load(), anchor: now().0, days: 7, mini: None, add_menu: false, form: None, error: None,
            scroll: ScrollHandle::new(), placed: false, links: HashMap::new(), notes: vec![], detail: None, choosing: false, picking: None, find, ext: Default::default(),
            name, kind, place, start, end, course, date, time, until, _subs: subs,
        }
    }

    /// The window came forward: what another window or device wrote meanwhile (not while a form is open).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.form.is_some() || self.ext.form.is_some() { return }
        // Google and iCloud may have changed meanwhile.
        self.ext_refresh(true);
        cx.notify();
        let d = model::load();
        if d != self.data { self.data = d; cx.notify(); }
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = model::save(&self.data) { log::warn!("schedule: save failed: {e}"); }
        cx.notify();
    }

    fn set(&self, f: &Entity<TextField>, s: &str, cx: &mut Context<Self>) { f.update(cx, |f, cx| f.set_text(s, cx)); }
    fn text(&self, f: &Entity<TextField>, cx: &App) -> String { f.read(cx).text.trim().to_string() }

    fn first_day(&self) -> Day { if self.days == 7 { self.anchor - model::weekday_of(self.anchor) as i64 } else { self.anchor } }
    /// Show `day` (a note's chip): its week, brought to `start` when it has one.
    pub fn goto(&mut self, day: Day, cx: &mut Context<Self>) { self.anchor = day; self.mini = None; self.placed = false; self.detail = None; cx.notify(); }

    fn open_detail(&mut self, link: Link, cx: &mut Context<Self>) { self.detail = Some(link); self.choosing = false; self.add_menu = false; cx.notify(); }

    /// 关联到课表 for `note` (its header's calendar button).
    pub fn pick_for(&mut self, note: String, window: &mut Window, cx: &mut Context<Self>) {
        self.picking = Some(note);
        self.data = model::load();
        self.read_around(now().0);
        let f = self.find.clone(); f.update(cx, |f, cx| f.set_text("", cx));
        window.focus(&f.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    fn go(&mut self, by: i64, cx: &mut Context<Self>) { self.anchor += by * self.days as i64; self.mini = None; cx.notify(); }
    fn go_today(&mut self, cx: &mut Context<Self>) { self.anchor = now().0; self.mini = None; self.placed = false; cx.notify(); }

    // ---- forms

    fn open_class(&mut self, id: Option<String>, day: u8, hour: Option<u16>, window: &mut Window, cx: &mut Context<Self>) {
        let c = id.as_ref().and_then(|id| self.data.classes.iter().find(|c| &c.id == id)).cloned();
        let c = c.unwrap_or_else(|| {
            let start = hour.unwrap_or(9) * 60;
            Class { day, start, end: (start + 90).min(24 * 60 - 1), color: crate::theme::FOLDER_COLORS[(self.data.courses().len() + 5) % 7].into(), ..Default::default() }
        });
        self.set(&self.name, &c.course, cx); self.set(&self.kind, &c.kind, cx); self.set(&self.place, &c.place, cx);
        self.set(&self.start, &model::hm(c.start), cx); self.set(&self.end, &model::hm(c.end), cx);
        self.form = Some(Form::Class { id, day: c.day, color: c.color });
        self.error = None;
        self.add_menu = false;
        window.focus(&self.name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    fn open_item(&mut self, id: Option<String>, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let i = id.as_ref().and_then(|id| self.data.items.iter().find(|i| &i.id == id)).cloned().unwrap_or(Item { kind, ..Default::default() });
        self.set(&self.name, &i.title, cx); self.set(&self.course, &i.course, cx);
        self.set(&self.date, &if id.is_some() { let (y, m, d) = model::ymd(i.day); format!("{y}-{m:02}-{d:02}") } else { String::new() }, cx);
        self.set(&self.time, &i.time.map(model::hm).unwrap_or_default(), cx);
        self.set(&self.until, &i.end.map(model::hm).unwrap_or_default(), cx);
        self.form = Some(Form::Item { id, kind: i.kind, done: i.done });
        self.error = None;
        self.add_menu = false;
        window.focus(&self.name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) { self.form = None; self.error = None; cx.notify(); }

    fn save_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.clone() else { return };
        let name = self.text(&self.name, cx);
        let time_of = |s: String| -> Result<Option<Minute>, ()> { if s.is_empty() { Ok(None) } else { model::parse_time(&s).map(Some).ok_or(()) } };
        const BAD_TIME: &str = "Times look like 9:00, 14:30 or 2pm.";
        match form {
            Form::Class { id, day, color } => {
                if name.is_empty() { self.error = Some("Give the class a course name."); return cx.notify() }
                let (Some(start), Some(end)) = (model::parse_time(&self.text(&self.start, cx)), model::parse_time(&self.text(&self.end, cx))) else { self.error = Some(BAD_TIME); return cx.notify() };
                if end <= start { self.error = Some("The class has to end after it starts."); return cx.notify() }
                let c = Class { id: id.clone().unwrap_or_else(model::new_id), course: name, kind: self.text(&self.kind, cx), place: self.text(&self.place, cx), color, day, start, end };
                match id.and_then(|id| self.data.classes.iter_mut().find(|x| x.id == id)) { Some(x) => *x = c, None => self.data.classes.push(c) }
            }
            Form::Item { id, kind, done } => {
                if name.is_empty() { self.error = Some("Give it a title."); return cx.notify() }
                let Some(day) = model::parse_day(&self.text(&self.date, cx), now().0) else { self.error = Some("Dates look like fri, tomorrow, 3.12 or 2026-12-03."); return cx.notify() };
                let (Ok(time), Ok(end)) = (time_of(self.text(&self.time, cx)), time_of(self.text(&self.until, cx))) else { self.error = Some(BAD_TIME); return cx.notify() };
                if let (Some(t), Some(e)) = (time, end) && e <= t { self.error = Some("It has to end after it starts."); return cx.notify() }
                let end = if kind == Kind::Assignment || time.is_none() { None } else { end };
                let i = Item { id: id.clone().unwrap_or_else(model::new_id), title: name, course: self.text(&self.course, cx), kind, day, time, end, done };
                match id.and_then(|id| self.data.items.iter_mut().find(|x| x.id == id)) { Some(x) => *x = i, None => self.data.items.push(i) }
                // Show where it went.
                self.anchor = day; self.mini = None;
            }
        }
        self.form = None;
        self.commit(cx);
    }

    fn delete_form(&mut self, cx: &mut Context<Self>) {
        match self.form.take() {
            Some(Form::Class { id: Some(id), .. }) => self.data.classes.retain(|c| c.id != id),
            Some(Form::Item { id: Some(id), .. }) => self.data.items.retain(|i| i.id != id),
            _ => {}
        }
        self.commit(cx);
    }

    fn toggle_done(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(i) = self.data.items.iter_mut().find(|i| i.id == id) { i.done = !i.done; }
        self.commit(cx);
    }

    // ---- the sidebar's card

    /// Two lines for the sidebar: the class on now (or the next one) and the nearest thing due.
    /// None while the schedule is empty.
    pub fn card(&self) -> Option<Div> {
        if self.data.classes.is_empty() && self.data.items.is_empty() { return None }
        let (day, min, _) = now();
        let nn = self.data.now_next(day, min);
        let line = |bar: Hsla, head: String, sub: String| div().flex().items_center().gap(px(9.))
            .child(div().w(px(3.)).h(px(28.)).flex_none().rounded_full().bg(bar))
            .child(div().min_w_0().flex().flex_col()
                .child(div().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(head))
                .child(div().text_size(px(11.)).text_color(color::muted_foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(sub)));
        let mut lines = vec![];
        if let Some(c) = nn.now {
            lines.push(line(ink(&c.color), c.course.clone(), join(&[&"Now".to_string(), &format!("until {}", model::clock(c.end)), &c.place])));
        } else if let Some((d, c)) = nn.next {
            let when = if d == day { model::until((day, min), (d, c.start)) } else { format!("{} {}", model::day_label(d), model::clock(c.start)) };
            lines.push(line(ink(&c.color), c.course.clone(), join(&[&when, &c.place])));
        }
        if let Some(i) = self.data.upcoming(day, OVERDUE_DAYS).first() {
            let late = i.day < day;
            let bar = if late { now_red() } else { self.data.color_of(&i.course).map(ink).unwrap_or_else(color::muted_foreground) };
            lines.push(line(bar, i.title.clone(), format!("Due {}", model::until((day, min), (i.day, i.time.unwrap_or(23 * 60 + 59))))));
        }
        if lines.is_empty() { return None }
        Some(div().flex().flex_col().gap(px(8.)).children(lines))
    }

    // ---- the page

    /// The page's one header row, in the window's title bar: the month, then the controls.
    pub fn header(&self, cx: &mut Context<Self>) -> Div {
        use crate::ui::TitleBarExt;
        let today = now().0;
        let first = self.first_day();
        let (y, m, _) = model::ymd(first);
        let (y2, m2, _) = model::ymd(first + self.days as i64 - 1);
        let month = if (y, m) == (y2, m2) { model::MONTHS_LONG[m as usize - 1].to_string() } else { format!("{} – {}", &model::MONTHS_LONG[m as usize - 1][..3], &model::MONTHS_LONG[m2 as usize - 1][..3]) };
        let showing_today = (first..first + self.days as i64).contains(&today);
        let add = |id: &'static str, glyph: &'static str, label: &'static str| MenuItem::new(id).icon(glyph).label(label);
        let menu = Popover::new("sch-add-menu").menu().w(px(design::menu_w::SM))
            .on_mouse_down_out(cx.listener(|p, _, _, cx| { p.add_menu = false; cx.notify(); }))
            .child(add("sch-new-class", "calendar", "Class").on_click(cx.listener(|p, _, window, cx| p.open_class(None, model::weekday_of(p.anchor), None, window, cx))))
            .child(add("sch-new-assignment", "square-pen", "Assignment").on_click(cx.listener(|p, _, window, cx| p.open_item(None, Kind::Assignment, window, cx))))
            .child(add("sch-new-exam", "star", "Exam").on_click(cx.listener(|p, _, window, cx| p.open_item(None, Kind::Exam, window, cx))))
            .child(add("sch-new-event", "plus", "Event").on_click(cx.listener(|p, _, window, cx| p.open_item(None, Kind::Event, window, cx))))
            // An event in Google or iCloud, when a calendar of this Mac takes one.
            .when(!self.writable_calendars().is_empty(), |m| m.child(MenuItem::new("sch-new-ext").icon("globe").label("Calendar event")
                .description("In Google or iCloud").on_click(cx.listener(|p, _, window, cx| { let d = p.anchor; p.open_ext_form(None, d, None, window, cx) }))));
        // The page header puts a spacer after its title (`page_header_with`): this row takes all the room.
        let mut bar = div().flex_1().min_w_0().h_full().flex().items_center().gap(px(6.));
        bar.style().flex_grow = Some(1000.);
        bar.child(div().min_w_0().flex().items_baseline().gap(px(7.)).overflow_hidden().whitespace_nowrap()
                .child(div().text_size(px(21.)).line_height(px(26.)).font_weight(FontWeight::BOLD).text_color(color::foreground()).child(month))
                .child(div().text_size(px(21.)).line_height(px(26.)).font_weight(FontWeight::NORMAL).text_color(color::muted_foreground()).child(y2.to_string())))
            .child(div().flex_1())
            .child(Segmented::new().w(px(124.))
                .child(Segment::new("sch-day", "Day").no_drag().selected(self.days == 1).on_click(cx.listener(|p, _, _, cx| { p.days = 1; cx.notify(); })))
                .child(Segment::new("sch-week", "Week").no_drag().selected(self.days == 7).on_click(cx.listener(|p, _, _, cx| { p.days = 7; cx.notify(); }))))
            .child(div().w(px(6.)))
            .child(Button::ghost("sch-prev").no_drag().icon("chevron-left").on_click(cx.listener(|p, _, _, cx| p.go(-1, cx))))
            .child(Button::outline("sch-today").no_drag().sized(ButtonSize::Sm).label("Today").disabled(showing_today && (self.days == 7 || self.anchor == today)).on_click(cx.listener(|p, _, _, cx| p.go_today(cx))))
            .child(Button::ghost("sch-next").no_drag().icon("chevron-right").on_click(cx.listener(|p, _, _, cx| p.go(1, cx))))
            .child(div().w(px(6.)))
            .child(div().relative()
                .child(Button::primary("sch-add").no_drag().sized(ButtonSize::Sm).icon("plus").label("New").on_click(cx.listener(|p, _, _, cx| { p.add_menu = !p.add_menu; cx.notify(); })))
                .when(self.add_menu, |d| d.child(div().absolute().right_0().top_full().child(deferred(anchored().snap_to_window_with_margin(px(8.)).child(div().pt(px(6.)).child(menu))).with_priority(2)))))
    }

    /// What one day of the grid draws: its classes, and the exams and events that have a time.
    fn blocks_on(&self, d: Day) -> Vec<Block> {
        let mut v: Vec<Block> = self.data.classes_on(model::weekday_of(d)).into_iter().map(|c| Block {
            link: Link::Class { id: c.id.clone(), day: d }, start: c.start, end: c.end, title: c.course.clone(), sub: join(&[&c.kind, &c.place]), accent: ink(&c.color), fill: wash(&c.color), done: false,
        }).collect();
        v.extend(self.data.items_on(d).into_iter().filter(|i| i.time.is_some() && i.kind != Kind::Assignment).map(|i| {
            let s = i.time.unwrap_or(0);
            let c = self.data.color_of(&i.course).unwrap_or(if i.kind == Kind::Exam { "red" } else { "gray" });
            Block { link: Link::Item { id: i.id.clone() }, start: s, end: i.end.unwrap_or((s + DEFAULT_LEN).min(24 * 60)), title: i.title.clone(), sub: join(&[&i.kind.label().to_string(), &i.course]),
                accent: ink(c), fill: wash(c), done: i.done }
        }));
        // Google, iCloud …: the calendars of this Mac (`external`).
        v.extend(self.ext_blocks(d));
        v
    }

    /// What sits in the all-day row on a day: deadlines, and anything without a time.
    fn all_day_on(&self, d: Day) -> Vec<&Item> {
        self.data.items_on(d).into_iter().filter(|i| i.time.is_none() || i.kind == Kind::Assignment).collect()
    }

    fn day_heads(&self, today: Day, cx: &mut Context<Self>) -> Div {
        let first = self.first_day();
        let wide = self.days == 1;
        let any_all_day = (0..self.days as i64).any(|k| !self.all_day_on(first + k).is_empty() || !self.ext_all_day(first + k).is_empty());
        let heads = div().flex().child(div().w(px(GUTTER_W)).flex_none())
            .children((0..self.days as i64).map(|k| {
                let d = first + k;
                let (_, _, dd) = model::ymd(d);
                let wd = model::weekday_of(d) as usize;
                let is_today = d == today;
                let name = if wide { ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][wd].to_string() } else { model::WEEKDAYS[wd].to_uppercase() };
                div().id(("sch-head", k as usize)).flex_1().min_w_0().h(px(44.)).flex().items_center().gap(px(6.)).px(px(10.)).cursor_pointer()
                    .when(!wide, |d| d.justify_center())
                    .on_click(cx.listener(move |p, _, _, cx| { p.anchor = d; p.days = 1; cx.notify(); }))
                    .child(div().text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(if is_today { now_red() } else { color::muted_foreground() }).child(name))
                    .child(div().min_w(px(28.)).h(px(28.)).px(px(4.)).rounded_full().flex().items_center().justify_center()
                        .text_size(px(16.)).font_weight(if is_today { FontWeight::BOLD } else { FontWeight::MEDIUM })
                        .when(is_today, |d| d.bg(now_red()).text_color(gpui::white()))
                        .when(!is_today, |d| d.text_color(color::foreground()))
                        .child(dd.to_string()))
            }));
        let all_day = any_all_day.then(|| div().flex().border_t_1().border_color(color::border()).py(px(4.))
            .child(div().w(px(GUTTER_W)).flex_none().pr(px(10.)).pt(px(3.)).flex().justify_end().text_size(px(10.5)).text_color(color::muted_foreground()).child("all-day"))
            .children((0..self.days as i64).map(|k| {
                let d = first + k;
                div().flex_1().min_w_0().px(px(3.)).flex().flex_col().gap(px(2.)).children(self.all_day_on(d).into_iter().map(|i| {
                    let c = self.data.color_of(&i.course).unwrap_or(if i.kind == Kind::Exam { "red" } else { "gray" }).to_string();
                    let id = i.id.clone();
                    let kind = i.kind;
                    div().id(SharedString::from(format!("allday-{}", i.id))).h(px(20.)).px(px(6.)).rounded(px(5.)).bg(wash(&c)).cursor_pointer()
                        .flex().items_center().gap(px(5.)).overflow_hidden().hover(|s| s.opacity(0.85))
                        .when(i.done, |d| d.opacity(0.5))
                        .child(div().size(px(6.)).flex_none().rounded_full().bg(ink(&c)))
                        .child(div().min_w_0().text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis()
                            .when(i.done, |d| d.line_through()).child(i.title.clone()))
                        .when_some(i.time.filter(|_| wide), |d, t| d.child(div().flex_none().text_size(px(10.5)).text_color(color::muted_foreground()).child(model::clock(t))))
                        .on_click(cx.listener(move |p, _, _, cx| { let _ = kind; p.open_detail(Link::Item { id: id.clone() }, cx) }))
                })).children(self.ext_all_day(d).into_iter().map(|e| self.ext_chip(e, cx)).collect::<Vec<_>>())
            })));
        div().flex_none().flex().flex_col().border_b_1().border_color(color::border()).child(heads).children(all_day)
    }

    fn grid(&mut self, today: Day, minute: Minute, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let first = self.first_day();
        let h = 24. * HOUR_H;
        // Opening (and 今天): the time now a little below the top, once the grid has a size.
        if !self.placed {
            if self.scroll.max_offset().y > px(0.) {
                let y = ((minute as f32 / 60. - LEAD_HOURS) * HOUR_H).max(0.);
                self.scroll.set_offset(point(px(0.), px(-y)));
                self.placed = true;
            } else { window.request_animation_frame(); }
        }
        let gutter = div().w(px(GUTTER_W)).flex_none().relative().h(px(h))
            .children((1..24u16).map(|hr| div().absolute().top(px(hr as f32 * HOUR_H - 7.)).right(px(10.)).text_size(px(10.5)).text_color(color::muted_foreground()).child(model::hour_label(hr))))
            // The time now, in red over its hour's label.
            .when((first..first + self.days as i64).contains(&today), |g| g.child(
                div().absolute().top(px(minute as f32 / 60. * HOUR_H - 9.)).right(px(4.)).h(px(18.)).px(px(6.)).rounded_full().bg(now_red())
                    .flex().items_center().text_size(px(10.5)).font_weight(FontWeight::SEMIBOLD).text_color(gpui::white()).child(model::clock(minute))));
        let mut columns = vec![];
        for k in 0..self.days as i64 {
            let d = first + k;
            let wd = model::weekday_of(d);
            let blocks = self.blocks_on(d);
            let places = model::lay_out(&blocks.iter().map(|b| (b.start, b.end)).collect::<Vec<_>>());
            let past_day = d < today;
            let col = div().flex_1().min_w_0().relative().h(px(h)).border_l_1().border_color(color::border())
                .when(wd >= 5, |c| c.bg(color::muted().opacity(0.35)))
                // An hour's slot: hovering hints, a click makes a class there.
                .children((0..24u16).map(|hr| {
                    let group = SharedString::from(format!("slot-{k}-{hr}"));
                    div().id(("slot", (k as usize) * 32 + hr as usize)).group(group.clone()).absolute().left_0().right_0().top(px(hr as f32 * HOUR_H)).h(px(HOUR_H))
                        .when(hr > 0, |s| s.border_t_1().border_color(color::border().opacity(0.7)))
                        .child(div().absolute().top(px(4.)).left(px(6.)).text_size(px(10.5)).text_color(color::muted_foreground()).opacity(0.)
                            .group_hover(group, |s| s.opacity(1.)).child(format!("+ {}", model::clock(hr * 60))))
                        .on_click(cx.listener(move |p, _, window, cx| p.open_class(None, wd, Some(hr), window, cx)))
                }))
                .children(blocks.into_iter().zip(places).map(|(b, (c, n))| {
                    let top = b.start as f32 / 60. * HOUR_H;
                    let height = ((b.end - b.start) as f32 / 60. * HOUR_H).max(22.);
                    let over = past_day || (d == today && b.end <= minute);
                    let on = d == today && b.start <= minute && minute < b.end;
                    let link = b.link.clone();
                    let noted = self.links.contains_key(&link.key());
                    let w = 1. / n as f32;
                    let (accent, fill) = (b.accent, b.fill);
                    div().id(SharedString::from(format!("blk-{}", link.key()))).occlude().absolute().top(px(top + 1.)).h(px(height - 2.))
                        .left(relative(c as f32 * w)).w(relative(w)).pl(px(2.)).pr(px(3.))
                        .child(div().size_full().rounded(px(7.)).bg(fill).border_l(px(3.)).border_color(accent).pl(px(7.)).pr(px(6.)).py(px(4.))
                            .overflow_hidden().cursor_pointer().flex().flex_col().gap(px(1.))
                            .when(over, |d| d.opacity(0.55))
                            .when(on, |d| d.border_1().border_l(px(3.)).border_color(accent))
                            .hover(|s| s.bg(fill.opacity((fill.a * 1.6).min(1.))))
                            .child(div().text_size(px(12.)).line_height(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(accent).overflow_hidden().whitespace_nowrap().text_ellipsis()
                                .when(b.done, |d| d.line_through()).child(b.title))
                            .when(height > 36., |d| d.child(div().text_size(px(11.)).line_height(px(14.)).text_color(color::muted_foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis()
                                .child(model::clock(b.start))))
                            .when(height > 54. && !b.sub.is_empty(), |d| d.child(div().text_size(px(11.)).line_height(px(14.)).text_color(color::muted_foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(b.sub))))
                        .when(noted, |d| d.child(div().absolute().top(px(5.)).right(px(7.)).child(crate::ui::icon("doc", 11., accent))))
                        .on_click(cx.listener(move |p, _, _, cx| p.open_detail(link.clone(), cx)))
                }))
                // The time now: a red line with a dot where it meets the day.
                .when(d == today, |col| {
                    let y = minute as f32 / 60. * HOUR_H;
                    col.child(div().absolute().left_0().right_0().top(px(y - 0.75)).h(px(1.5)).bg(now_red()))
                        .child(div().absolute().left(px(-5.)).top(px(y - 5.)).size(px(10.)).rounded_full().bg(now_red()))
                });
            columns.push(col);
        }
        div().id("sch-grid").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.scroll)
            .child(div().flex().pt(px(10.)).pb(px(10.)).child(gutter).children(columns))
    }

    /// A month at a glance: today in red, the range shown lit, a dot under days with something due.
    fn mini_month(&self, today: Day, cx: &mut Context<Self>) -> Div {
        let (ay, am, _) = model::ymd(self.anchor);
        let (y, m) = self.mini.unwrap_or((ay, am));
        let first_of = crate::time::days_from_civil(y, m, 1);
        let start = first_of - model::weekday_of(first_of) as i64;
        let shown = self.first_day()..self.first_day() + self.days as i64;
        let step = move |p: &mut Self, by: i64, cx: &mut Context<Self>| {
            let (ay, am, _) = model::ymd(p.anchor);
            let (y, m) = p.mini.unwrap_or((ay, am));
            let n = y * 12 + m as i64 - 1 + by;
            p.mini = Some((n.div_euclid(12), (n.rem_euclid(12) + 1) as u32));
            cx.notify();
        };
        div().flex().flex_col().gap(px(4.))
            .child(div().flex().items_center()
                .child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(color::foreground()).child(format!("{} {y}", model::MONTHS_LONG[m as usize - 1])))
                .child(Button::ghost("mini-prev").icon("chevron-left").on_click(cx.listener(move |p, _, _, cx| step(p, -1, cx))))
                .child(Button::ghost("mini-next").icon("chevron-right").on_click(cx.listener(move |p, _, _, cx| step(p, 1, cx)))))
            .child(div().flex().children(["M", "T", "W", "T", "F", "S", "S"].map(|l| div().flex_1().flex().justify_center().text_size(px(10.5)).font_weight(FontWeight::MEDIUM).text_color(color::muted_foreground()).child(l))))
            .children((0..6i64).map(|row| div().flex().children((0..7i64).map(|col| {
                let d = start + row * 7 + col;
                let (_, dm, dd) = model::ymd(d);
                let (is_today, in_month, lit) = (d == today, dm == m, shown.contains(&d));
                let due = self.data.items.iter().any(|i| i.day == d && !i.done);
                let (left_end, right_end) = (col == 0 || d == shown.start, col == 6 || d == shown.end - 1);
                div().id(("mini", (row * 7 + col) as usize)).flex_1().h(px(32.)).flex().flex_col().items_center().justify_center().cursor_pointer()
                    .when(lit, |c| c.bg(color::muted()).when(left_end, |c| c.rounded_l(px(8.))).when(right_end, |c| c.rounded_r(px(8.))))
                    .on_click(cx.listener(move |p, _, _, cx| { p.anchor = d; p.mini = None; cx.notify(); }))
                    .child(div().size(px(22.)).rounded_full().flex().items_center().justify_center().text_size(px(12.))
                        .when(is_today, |c| c.bg(now_red()).text_color(gpui::white()).font_weight(FontWeight::BOLD))
                        .when(!is_today, |c| c.text_color(if in_month { color::foreground() } else { color::muted_foreground().opacity(0.6) }).hover(|s| s.bg(color::accent())))
                        .child(dd.to_string()))
                    .child(div().size(px(4.)).rounded_full().when(due, |c| c.bg(if is_today { now_red() } else { color::muted_foreground() })))
            }))))
    }

    fn panel(&self, today: Day, minute: Minute, cx: &mut Context<Self>) -> Div {
        let heading = |s: &'static str| div().mt(px(22.)).mb(px(8.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(color::foreground()).child(s);
        let muted = |s: String| div().text_size(px(12.5)).line_height(px(18.)).text_color(color::muted_foreground()).child(s);
        let nn = self.data.now_next(today, minute);
        let mut p = div().p(px(20.)).pt(px(8.)).flex().flex_col().child(self.mini_month(today, cx));
        if self.data.classes.is_empty() && self.data.items.is_empty() {
            let cals = self.calendars_section(cx);
            return p.child(heading("Your week is clear"))
                .child(muted("Add your timetable once and it repeats every week. Click any hour in the grid, or use New.".into()))
                .child(div().mt(px(12.)).flex().gap(px(8.))
                    .child(Button::primary("empty-class").sized(ButtonSize::Sm).icon("plus").label("Add a class").on_click(cx.listener(|p, _, window, cx| p.open_class(None, model::weekday_of(p.anchor), None, window, cx))))
                    .child(Button::outline("empty-item").sized(ButtonSize::Sm).label("Add a deadline").on_click(cx.listener(|p, _, window, cx| p.open_item(None, Kind::Assignment, window, cx)))))
                .children(cals);
        }
        // Up next: one card, with how far through the class is when it is on.
        let card = |c: &Class, label: String, sub: String, progress: Option<f32>| div().p(px(14.)).rounded(px(12.)).bg(wash(&c.color)).flex().flex_col().gap(px(3.))
            .child(div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(ink(&c.color)).child(label))
            .child(div().text_size(px(17.)).line_height(px(22.)).font_weight(FontWeight::BOLD).text_color(color::foreground()).child(c.course.clone()))
            .child(div().text_size(px(12.)).text_color(color::muted_foreground()).child(sub))
            .when_some(progress, |d, f| d.child(div().mt(px(8.)).h(px(4.)).rounded_full().bg(ink(&c.color).opacity(0.2))
                .child(div().h_full().w(relative(f.clamp(0., 1.))).rounded_full().bg(ink(&c.color)))));
        p = p.child(heading("Up Next"));
        p = match (nn.now, nn.next) {
            (Some(c), _) => p.child(card(c, format!("NOW · ENDS {}", model::until((today, minute), (today, c.end)).to_uppercase()),
                join(&[&format!("{} – {}", model::clock(c.start), model::clock(c.end)), &c.kind, &c.place]),
                Some(minute.saturating_sub(c.start) as f32 / (c.end - c.start).max(1) as f32))),
            (None, Some((d, c))) => {
                let when = if d == today { model::until((today, minute), (d, c.start)).to_uppercase() } else { model::day_label(d).to_uppercase() };
                p.child(card(c, when, join(&[&model::clock(c.start), &c.kind, &c.place]), None))
            }
            (None, None) => p.child(muted("No classes coming up.".into())),
        };
        if let (Some(_), Some((d, c))) = (nn.now, nn.next) {
            p = p.child(div().mt(px(8.)).flex().items_center().gap(px(8.)).child(div().size(px(7.)).rounded_full().bg(ink(&c.color)))
                .child(muted(format!("Then {} · {}", c.course, if d == today { model::clock(c.start) } else { format!("{} {}", model::day_label(d), model::clock(c.start)) }))));
        }
        // Today, in order.
        let classes = self.data.classes_on(model::weekday_of(today));
        if !classes.is_empty() {
            p = p.child(heading("Today")).children(classes.into_iter().map(|c| {
                let over = c.end <= minute;
                div().flex().items_center().gap(px(10.)).py(px(5.)).when(over, |d| d.opacity(0.45))
                    .child(div().w(px(3.)).h(px(28.)).rounded_full().bg(ink(&c.color)))
                    .child(div().min_w_0().flex_1().flex().flex_col()
                        .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(c.course.clone()))
                        .child(div().text_size(px(11.5)).text_color(color::muted_foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis()
                            .child(join(&[&format!("{} – {}", model::clock(c.start), model::clock(c.end)), &c.place]))))
            }));
        }
        let due = self.data.upcoming(today, OVERDUE_DAYS);
        p = p.child(heading("Due"));
        if due.is_empty() { p = p.child(muted("Nothing due. Nice.".into())); }
        let cals = self.calendars_section(cx);
        p.children(due.into_iter().take(12).map(|i| {
            let late = i.day < today || (i.day == today && i.time.is_some_and(|t| t < minute));
            let c = self.data.color_of(&i.course).unwrap_or("gray").to_string();
            let (tick, open, kind) = (i.id.clone(), i.id.clone(), i.kind);
            let left = model::until((today, minute), (i.day, i.time.unwrap_or(23 * 60 + 59)));
            div().id(SharedString::from(format!("due-row-{}", i.id))).flex().items_start().gap(px(10.)).py(px(7.)).px(px(8.)).mx(px(-8.)).rounded(px(8.)).cursor_pointer()
                .hover(|s| s.bg(color::accent_hover()))
                .on_click(cx.listener(move |p, _, _, cx| { let _ = kind; p.open_detail(Link::Item { id: open.clone() }, cx) }))
                .child(div().id(SharedString::from(format!("due-tick-{}", i.id))).mt(px(1.)).occlude().child(Checkbox::new(i.done))
                    .on_click(cx.listener(move |p, _, _, cx| { cx.stop_propagation(); p.toggle_done(&tick, cx); })))
                .child(div().min_w_0().flex_1().flex().flex_col().gap(px(2.))
                    .child(div().flex().items_center().gap(px(6.))
                        .child(div().min_w_0().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(i.title.clone()))
                        .when(i.kind != Kind::Assignment, |d| d.child(design::Badge::new(i.kind.label()))))
                    .child(div().flex().items_center().gap(px(6.))
                        .when(!i.course.is_empty(), |d| d.child(div().size(px(7.)).flex_none().rounded_full().bg(ink(&c))).child(div().text_size(px(11.5)).text_color(color::muted_foreground()).child(i.course.clone())))
                        .child(div().text_size(px(11.5)).font_weight(if late { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }).text_color(if late { now_red() } else { color::muted_foreground() })
                            .child(format!("{}{} · {left}", model::day_label(i.day), i.time.map(|t| format!(" {}", model::clock(t))).unwrap_or_default())))))
        })).children(cals)
    }

    fn field(&self, id: &'static str, label: &'static str, f: &Entity<TextField>, window: &Window, cx: &App) -> Div {
        let h = f.read(cx).focus_handle(cx);
        div().flex_1().min_w_0().flex().flex_col().gap(px(6.)).child(Label::new(label))
            .child(Field::new(id).focused(f.read(cx).is_focused(window)).cursor_text()
                .on_mouse_down(MouseButton::Left, move |_, window, cx| window.focus(&h, cx))
                .child(div().flex_1().min_w_0().overflow_hidden().child(f.clone())))
    }

    fn form(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.form.clone()?;
        let editing = matches!(&form, Form::Class { id: Some(_), .. } | Form::Item { id: Some(_), .. });
        let body = match &form {
            Form::Class { day, color: picked, .. } => div().flex().flex_col().gap(px(14.))
                .child(div().flex().gap(px(10.)).child(self.field("f-course", "Course", &self.name, window, cx)).child(self.field("f-kind", "Type", &self.kind, window, cx)))
                .child(div().flex().flex_col().gap(px(6.)).child(Label::new("Every"))
                    .child(Segmented::new().children((0..7u8).map(|k| Segment::new(("f-day", k as usize), model::WEEKDAYS[k as usize]).selected(k == *day)
                        .on_click(cx.listener(move |p, _, _, cx| { if let Some(Form::Class { day, .. }) = p.form.as_mut() { *day = k; } cx.notify(); }))))))
                .child(div().flex().gap(px(10.)).child(self.field("f-start", "Starts", &self.start, window, cx)).child(self.field("f-end", "Ends", &self.end, window, cx)))
                .child(self.field("f-place", "Room", &self.place, window, cx))
                .child(div().flex().flex_col().gap(px(6.)).child(Label::new("Colour"))
                    .child(div().flex().gap(px(4.)).children(crate::theme::FOLDER_COLORS.iter().enumerate().map(|(k, name)| {
                        let on = picked == name;
                        div().id(("f-color", k)).size(px(28.)).rounded_full().cursor_pointer().flex().items_center().justify_center()
                            .border(px(if on { 2. } else { 0. })).border_color(ink(name)).hover(|s| s.bg(color::accent()))
                            .child(div().size(px(18.)).rounded_full().bg(ink(name)))
                            .on_click(cx.listener(move |p, _, _, cx| { if let Some(Form::Class { color, .. }) = p.form.as_mut() { *color = (*name).into(); } cx.notify(); }))
                    })))),
            Form::Item { kind, done, .. } => {
                let courses = self.data.courses();
                let timed = *kind != Kind::Assignment;
                div().flex().flex_col().gap(px(14.))
                    .child(Segmented::new().children(Kind::ALL.iter().enumerate().map(|(k, x)| { let x = *x; Segment::new(("f-ikind", k), x.label()).selected(x == *kind)
                        .on_click(cx.listener(move |p, _, _, cx| { if let Some(Form::Item { kind, .. }) = p.form.as_mut() { *kind = x; } cx.notify(); })) })))
                    .child(self.field("f-title", "Title", &self.name, window, cx))
                    .child(div().flex().flex_col().gap(px(6.)).child(self.field("f-icourse", "Course", &self.course, window, cx))
                        .when(!courses.is_empty(), |d| d.child(div().flex().flex_wrap().gap(px(4.)).children(courses.into_iter().enumerate().map(|(k, (name, c))| {
                            let n = name.clone();
                            div().id(("f-pick", k)).px(px(9.)).py(px(3.)).rounded_full().bg(wash(&c)).cursor_pointer().text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(ink(&c)).child(name)
                                .on_click(cx.listener(move |p, _, _, cx| { let f = p.course.clone(); f.update(cx, |f, cx| f.set_text(&n, cx)); cx.notify(); }))
                        })))))
                    .child(div().flex().gap(px(10.))
                        .child(self.field("f-date", if timed { "Date" } else { "Due" }, &self.date, window, cx))
                        .child(self.field("f-time", if timed { "Starts" } else { "Time" }, &self.time, window, cx))
                        .when(timed, |d| d.child(self.field("f-until", "Ends", &self.until, window, cx))))
                    .when(editing, |d| d.child(div().id("f-done").flex().items_center().gap(px(8.)).cursor_pointer().child(Checkbox::new(*done)).child(Label::new("Done"))
                        .on_click(cx.listener(|p, _, _, cx| { if let Some(Form::Item { done, .. }) = p.form.as_mut() { *done = !*done; } cx.notify(); }))))
            }
        };
        let title = match (&form, editing) {
            (Form::Class { .. }, false) => "New class".to_string(),
            (Form::Class { .. }, true) => "Edit class".to_string(),
            (Form::Item { kind, .. }, false) => format!("New {}", kind.label().to_lowercase()),
            (Form::Item { kind, .. }, true) => format!("Edit {}", kind.label().to_lowercase()),
        };
        let footer = div().w_full().flex().items_center().gap(px(8.))
            .when(editing, |d| d.child(Button::ghost("f-delete").sized(ButtonSize::Default).icon("bin").label("Delete").on_click(cx.listener(|p, _, _, cx| p.delete_form(cx)))))
            .child(div().flex_1())
            .child(Button::outline("f-cancel").label("Cancel").on_click(cx.listener(|p, _, _, cx| p.close_form(cx))))
            .child(Button::primary("f-save").label("Save").on_click(cx.listener(|p, _, _, cx| p.save_form(cx))));
        Some(design::scrim().id("sch-scrim").occlude()
            .on_key_down(cx.listener(|p, ev: &KeyDownEvent, _, cx| if ev.keystroke.key == "escape" { p.close_form(cx); cx.stop_propagation(); }))
            .child(design::Dialog::new("sch-form").w(px(480.)).title(title)
                .closable(cx.listener(|p, _, _, cx| p.close_form(cx)))
                .child(body)
                .when_some(self.error, |d, e| d.child(div().text_size(px(12.5)).text_color(now_red()).child(e)))
                .footer(footer))
            .into_any_element())
    }
}

/// The parts that are there, joined with " · ".
fn join(parts: &[&String]) -> String { parts.iter().filter(|s| !s.is_empty()).map(|s| s.as_str()).collect::<Vec<_>>().join(" · ") }

impl SchedulePage {
    /// A note's row in a list: its title, opening it on a click.
    /// A linked note's row in the details card: opens it on a click; 取消关联 (shown on hover) unlinks it
    /// and leaves the note itself alone.
    fn note_row(&self, key: usize, id: String, title: String, cx: &mut Context<Self>) -> Stateful<Div> {
        let group = SharedString::from(format!("det-note-{key}"));
        let unlink = id.clone();
        div().id(("det-note", key)).group(group.clone()).h(px(34.)).pl(px(10.)).pr(px(4.)).rounded(px(8.)).flex().items_center().gap(px(10.)).cursor_pointer().hover(|s| s.bg(color::accent()))
            .child(crate::ui::icon("doc", 15., color::muted_foreground()))
            .child(div().min_w_0().flex_1().text_size(px(13.)).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(crate::settings::display_title(&title)))
            .child(div().id(("det-unlink", key)).occlude().h(px(26.)).px(px(8.)).rounded(px(6.)).flex().items_center().gap(px(4.))
                .text_size(px(12.)).text_color(color::muted_foreground()).opacity(0.).group_hover(group, |s| s.opacity(1.))
                .hover(|s| s.bg(color::popover()).text_color(color::foreground()))
                .child(crate::ui::icon("x", 12., color::muted_foreground())).child("Unlink")
                .on_click(cx.listener(move |_, _, _, cx| { cx.stop_propagation(); cx.emit(ScheduleEvent::Link { note: unlink.clone(), link: None }); cx.notify(); })))
            .on_click(cx.listener(move |p, _, _, cx| { p.detail = None; cx.emit(ScheduleEvent::Open(id.clone())); cx.notify(); }))
    }

    /// The notes `find` matches, `limit` at most, leaving out `skip`.
    fn matching_notes(&self, skip: &[(String, String)], limit: usize, cx: &App) -> Vec<(String, String)> {
        let q = self.find.read(cx).text.trim().to_lowercase();
        self.notes.iter().filter(|(id, t)| !skip.iter().any(|(s, _)| s == id) && (q.is_empty() || t.to_lowercase().contains(&q))).take(limit).cloned().collect()
    }

    /// The details card of a lecture, an item or a calendar event: when and where, its notes, and what
    /// to do with them.
    fn details(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let link = self.detail.clone()?;
        let (today, minute, _) = now();
        let notes = self.links.get(&link.key()).cloned().unwrap_or_default();
        // (title, colour, the line under it, the folder a new note goes in, the new note's title, the event to edit)
        let (title, accent, line, course, note_title, event) = match &link {
            Link::Class { id, day } => {
                let c = self.data.classes.iter().find(|c| &c.id == id)?;
                let on = *day == today && c.start <= minute && minute < c.end;
                let rel = if on { "on now".to_string() } else { model::until((today, minute), (*day, c.start)) };
                let when = format!("{} · {} – {}", model::day_label(*day), model::clock(c.start), model::clock(c.end));
                (c.course.clone(), ink(&c.color), join(&[&c.kind, &when, &c.place, &rel]), c.course.clone(), format!("{} – {}", c.course, model::day_label(*day)), None)
            }
            Link::Item { id } => {
                let i = self.data.items.iter().find(|i| &i.id == id)?;
                let t = match (i.time, i.end) { (Some(s), Some(e)) => format!(" · {} – {}", model::clock(s), model::clock(e)), (Some(s), None) => format!(" · {}", model::clock(s)), _ => String::new() };
                let c = self.data.color_of(&i.course).unwrap_or(if i.kind == Kind::Exam { "red" } else { "gray" });
                let rel = model::until((today, minute), (i.day, i.time.unwrap_or(0)));
                (i.title.clone(), ink(c), join(&[&i.kind.label().to_string(), &format!("{}{t}", model::day_label(i.day)), &rel]), i.course.clone(), i.title.clone(), None)
            }
            Link::Event { id, start } => {
                let e = self.ext_event(id, *start)?;
                let (accent, d0) = (self.ext_describe(id, *start).map(|(_, a)| a).unwrap_or_else(color::muted_foreground), model::local(e.start as u64).0);
                let rel = if e.start <= crate::time::now() as i64 && (crate::time::now() as i64) < e.end { "on now".to_string() } else { model::until((today, minute), (d0, model::local(e.start as u64).1)) };
                (e.title.clone(), accent, join(&[&self.ext_calendar_title(&e), &Self::ext_when(&e), &e.location, &rel]), String::new(), format!("{} – {}", e.title, model::day_label(d0)), Some(e))
            }
        };
        let d = model::Described { title, sub: line, color: String::new(), day: 0, start: None };
        let title = note_title;
        let editable = event.as_ref().is_none_or(|e| self.ext_writable(e));
        let (new_link, new_title, new_course) = (link.clone(), title.clone(), course.clone());
        let edit = link.clone();
        let mut body = div().flex().flex_col().gap(px(4.))
            .child(div().flex().items_center().gap(px(10.))
                .child(div().w(px(4.)).h(px(40.)).rounded_full().bg(accent))
                .child(div().min_w_0().flex().flex_col()
                    .child(div().text_size(px(19.)).line_height(px(24.)).font_weight(FontWeight::BOLD).text_color(color::foreground()).child(d.title.clone()))
                    .child(div().text_size(px(12.5)).text_color(color::muted_foreground()).child(d.sub.clone()))))
            .child(div().mt(px(14.)).mb(px(4.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(color::muted_foreground()).child("Notes"));
        if notes.is_empty() && !self.choosing {
            body = body.child(div().text_size(px(12.5)).text_color(color::muted_foreground()).child("No notes yet. Make one for this, or link one you already have."));
        }
        body = body.children(notes.iter().enumerate().map(|(k, (id, t))| self.note_row(k, id.clone(), t.clone(), cx)));
        if self.choosing {
            let unlink_target = link.clone();
            body = body.child(div().mt(px(6.)).flex().flex_col().gap(px(6.))
                .child(self.field("det-find", "Link a note", &self.find, window, cx))
                .child(div().id("det-choices").max_h(px(220.)).overflow_y_scroll().flex().flex_col()
                    .children(self.matching_notes(&notes, 40, cx).into_iter().enumerate().map(|(k, (id, t))| {
                        let (note, l) = (id.clone(), unlink_target.clone());
                        div().id(("det-choice", k)).h(px(32.)).px(px(10.)).rounded(px(8.)).flex().items_center().gap(px(10.)).cursor_pointer().hover(|s| s.bg(color::accent()))
                            .child(crate::ui::icon("plus", 14., color::muted_foreground()))
                            .child(div().min_w_0().flex_1().text_size(px(13.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(crate::settings::display_title(&t)))
                            .on_click(cx.listener(move |p, _, _, cx| { p.choosing = false; cx.emit(ScheduleEvent::Link { note: note.clone(), link: Some(l.clone()) }); cx.notify(); }))
                    }))));
        }
        let footer = div().w_full().flex().items_center().gap(px(8.))
            .when(editable, |d| d.child(Button::ghost("det-edit").sized(ButtonSize::Default).icon("square-pen").label("Edit")
                .on_click(cx.listener(move |p, _, window, cx| { p.detail = None; match &edit {
                    Link::Class { id, day } => p.open_class(Some(id.clone()), model::weekday_of(*day), None, window, cx),
                    Link::Item { id } => p.open_item(Some(id.clone()), Kind::Event, window, cx),
                    Link::Event { id, start } => { let e = p.ext_event(id, *start); let day = now().0; p.open_ext_form(e, day, None, window, cx) }
                } }))))
            .when(!editable, |d| d.child(div().text_size(px(12.)).text_color(color::muted_foreground()).child("Read-only calendar")))
            .child(div().flex_1())
            .when(!self.choosing, |d| d.child(Button::outline("det-link").label("Link a note").on_click(cx.listener(|p, _, window, cx| {
                p.choosing = true; let f = p.find.clone(); f.update(cx, |f, cx| f.set_text("", cx)); window.focus(&f.read(cx).focus_handle(cx), cx); cx.notify();
            }))))
            .child(Button::primary("det-new").icon("plus").label("New note").on_click(cx.listener(move |p, _, _, cx| {
                p.detail = None; cx.emit(ScheduleEvent::NewNote { link: new_link.clone(), title: new_title.clone(), course: new_course.clone() }); cx.notify();
            })));
        Some(design::scrim().id("det-scrim").occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|p, _, _, cx| { p.detail = None; cx.notify(); }))
            .on_key_down(cx.listener(|p, ev: &KeyDownEvent, _, cx| if ev.keystroke.key == "escape" { p.detail = None; cx.stop_propagation(); cx.notify(); }))
            .child(design::Dialog::new("det").w(px(440.)).occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .closable(cx.listener(|p, _, _, cx| { p.detail = None; cx.notify(); }))
                .child(body).footer(footer))
            .into_any_element())
    }

    /// 关联到课表, from a note's header: what this note was for, the lectures around now first.
    pub fn picker(&mut self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let note = self.picking.clone()?;
        let (today, minute, _) = now();
        let current = self.notes_link(&note);
        let q = self.find.read(cx).text.trim().to_lowercase();
        // Lectures and calendar events by how near now they are (the one on now first), then what is due.
        let now_unix = crate::time::now() as i64;
        let mut near: Vec<(i64, Link)> = self.data.link_choices(today, minute, 14).into_iter().map(|l| (match &l {
            Link::Class { id, day } => self.data.classes.iter().find(|c| &c.id == id).map_or(i64::MAX, |c| {
                let (s, e) = (external::unix_of(*day, c.start), external::unix_of(*day, c.end));
                if s <= now_unix && now_unix < e { -1 } else { (s - now_unix).abs() / 60 }
            }),
            _ => i64::MAX,
        }, l)).collect();
        near.extend(self.ext_choices());
        near.sort_by_key(|(d, _)| *d);
        let choices: Vec<(Link, model::Described, Hsla)> = near.into_iter()
            .filter_map(|(_, l)| self.describe(&l).map(|(d, a)| (l, d, a)))
            .filter(|(_, d, _)| q.is_empty() || d.title.to_lowercase().contains(&q) || d.sub.to_lowercase().contains(&q))
            .take(40).collect();
        let close = cx.listener(|p: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| { p.picking = None; cx.notify(); });
        let n = note.clone();
        let list = div().id("pick-list").max_h(px(320.)).overflow_y_scroll().flex().flex_col()
            .when(choices.is_empty(), |d| d.child(div().py(px(8.)).text_size(px(12.5)).text_color(color::muted_foreground())
                .child(if self.data.classes.is_empty() && self.data.items.is_empty() { "Your schedule is empty. Add classes on the Schedule page first." } else { "Nothing matches." })))
            .children(choices.into_iter().enumerate().map(|(k, (l, d, accent))| {
                let on = current.as_ref() == Some(&l);
                let (note, l2) = (note.clone(), l.clone());
                let is_now = match &l {
                    Link::Class { id, day } => *day == today && self.data.now_next(today, minute).now.is_some_and(|c| *id == c.id),
                    Link::Event { id, start } => self.ext_event(id, *start).is_some_and(|e| e.start <= now_unix && now_unix < e.end),
                    Link::Item { .. } => false,
                };
                div().id(("pick", k)).h(px(44.)).px(px(10.)).rounded(px(8.)).flex().items_center().gap(px(10.)).cursor_pointer()
                    .when(on, |d| d.bg(color::muted())).hover(|s| s.bg(color::accent()))
                    .child(div().w(px(3.)).h(px(26.)).rounded_full().bg(accent))
                    .child(div().min_w_0().flex_1().flex().flex_col()
                        .child(div().flex().items_center().gap(px(6.)).child(div().min_w_0().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(d.title.clone()))
                            .when(is_now, |d| d.child(design::Badge::destructive("Now"))))
                        .child(div().text_size(px(11.5)).text_color(color::muted_foreground()).child(d.sub.clone())))
                    .when(on, |d| d.child(crate::ui::icon("check", 15., color::foreground())))
                    .on_click(cx.listener(move |p, _, _, cx| { p.picking = None; cx.emit(ScheduleEvent::Link { note: note.clone(), link: Some(l2.clone()) }); cx.notify(); }))
            }));
        let footer = div().w_full().flex().items_center().gap(px(8.))
            .when(current.is_some(), |d| d.child(Button::ghost("pick-unlink").sized(ButtonSize::Default).label("Remove link")
                .on_click(cx.listener(move |p, _, _, cx| { p.picking = None; cx.emit(ScheduleEvent::Link { note: n.clone(), link: None }); cx.notify(); }))))
            .child(div().flex_1())
            .child(Button::outline("pick-cancel").label("Cancel").on_click(close));
        Some(design::scrim().id("pick-scrim").occlude()
            .on_key_down(cx.listener(|p, ev: &KeyDownEvent, _, cx| if ev.keystroke.key == "escape" { p.picking = None; cx.stop_propagation(); cx.notify(); }))
            .child(design::Dialog::new("pick").w(px(460.)).title("Link to the schedule").description("Which lecture, assignment, exam or calendar event was this note for?")
                .closable(cx.listener(|p, _, _, cx| { p.picking = None; cx.notify(); }))
                .child(div().flex().flex_col().gap(px(8.)).child(self.field("pick-find", "Search", &self.find, window, cx)).child(list))
                .footer(footer))
            .into_any_element())
    }

    /// What `link` points at, with its colour: lectures and items from the schedule, calendar events
    /// from EventKit. None once it is gone.
    pub fn describe(&self, link: &Link) -> Option<(model::Described, Hsla)> {
        match link {
            Link::Event { id, start } => self.ext_describe(id, *start),
            _ => self.data.describe(link).map(|d| { let a = ink(&d.color); (d, a) }),
        }
    }

    /// The link a note has, from `links`.
    fn notes_link(&self, note: &str) -> Option<Link> {
        self.links.iter().find(|(_, v)| v.iter().any(|(id, _)| id == note)).and_then(|(k, _)| Link::parse(k))
    }
}

impl Render for SchedulePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (today, minute, _) = now();
        // Google, iCloud …: read when the days shown move, or once a minute.
        self.ext_refresh(false);
        let form = self.form(window, cx).or_else(|| self.ext_form_view(window, cx));
        let details = if form.is_none() { self.details(window, cx) } else { None };
        let wide = f32::from(window.viewport_size().width) >= PANEL_MIN_WINDOW;
        let heads = self.day_heads(today, cx);
        let grid = self.grid(today, minute, window, cx);
        let panel = wide.then(|| self.panel(today, minute, cx));
        div().id("schedule").relative().size_full().flex().flex_col().font_family(crate::theme::ui_family()).text_color(color::foreground())
            .child(div().flex_1().min_h_0().flex()
                .child(div().flex_1().min_w_0().flex().flex_col().pl(px(8.)).child(heads).child(grid))
                .children(panel.map(|p| div().id("sch-panel").w(px(PANEL_W)).flex_none().h_full().overflow_y_scroll().border_l_1().border_color(color::border()).child(p))))
            .children(form)
            .children(details)
    }
}
