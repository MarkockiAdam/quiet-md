//! 课表 (fork feature): a student's week. Classes repeat weekly; assignments, exams and events sit on
//! one day. The page shows the week as a grid with a line at the time now, and beside it what is
//! on now, what is next, today, and what is due. The sidebar's card says the same in two lines.
//! The data lives in `model.rs`; this is its page, its own entity so the workspace only hosts it.

pub mod model;

use crate::design::{self, Button, ButtonSize, Checkbox, Field, Label, Segment, Segmented, color};
use crate::input::{TextField, TextFieldEvent};
use crate::settings::tr;
use gpui::{prelude::FluentBuilder, *};
use model::{Class, Day, Item, Kind, Minute, Schedule};
use std::time::Duration;

/// One hour in the week grid.
const HOUR_H: f32 = 44.;
const GUTTER_W: f32 = 44.;
const PANEL_W: f32 = 300.;
/// How far back an unfinished item still shows under 待完成 (overdue).
const OVERDUE_DAYS: i64 = 14;

#[derive(Clone, PartialEq)]
enum Form {
    Class { id: Option<String>, day: u8, color: String },
    Item { id: Option<String>, kind: Kind, done: bool },
}

pub struct SchedulePage {
    pub data: Schedule,
    /// Weeks from this one (‹ ›).
    week: i64,
    form: Option<Form>,
    error: Option<&'static str>,
    name: Entity<TextField>,
    kind: Entity<TextField>,
    place: Entity<TextField>,
    start: Entity<TextField>,
    end: Entity<TextField>,
    course: Entity<TextField>,
    date: Entity<TextField>,
    time: Entity<TextField>,
    _subs: Vec<Subscription>,
}

/// Now, as the schedule counts it: (day, minute, weekday).
fn now() -> (Day, Minute, u8) { model::local(crate::time::now()) }

/// A course colour's ink (text, edges) and its wash (block fill).
fn ink(c: &str) -> Hsla { crate::theme::text_color(c).unwrap_or_else(color::muted_foreground) }
fn wash(c: &str) -> Hsla { ink(c).opacity(if crate::theme::dark() { 0.22 } else { 0.14 }) }

impl SchedulePage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let field = |cx: &mut Context<Self>, hint: &'static str| cx.new(|cx| TextField::new(hint, cx));
        let (name, kind, place, start, end) = (field(cx, "Calculus"), field(cx, "Lecture"), field(cx, "Room B12"), field(cx, "9:00"), field(cx, "10:30"));
        let (course, date, time) = (field(cx, "Calculus"), field(cx, "fri, 3.12, 2026-12-03"), field(cx, "23:59 (optional)"));
        let mut subs = vec![];
        for f in [&name, &kind, &place, &start, &end, &course, &date, &time] {
            subs.push(cx.subscribe(f, |p: &mut Self, _, ev: &TextFieldEvent, cx| match ev {
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
        SchedulePage { data: model::load(), week: 0, form: None, error: None, name, kind, place, start, end, course, date, time, _subs: subs }
    }

    /// Another library came forward: its schedule.
    /// The window came forward: what another window or device wrote meanwhile (not while a form is open).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.form.is_some() { return }
        let d = model::load();
        if d != self.data { self.data = d; cx.notify(); }
    }

    fn commit(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = model::save(&self.data) { log::warn!("schedule: save failed: {e}"); }
        cx.notify();
    }

    fn set(&self, f: &Entity<TextField>, s: &str, cx: &mut Context<Self>) { f.update(cx, |f, cx| f.set_text(s, cx)); }
    fn text(&self, f: &Entity<TextField>, cx: &App) -> String { f.read(cx).text.trim().to_string() }

    // ---- forms

    fn open_class(&mut self, id: Option<String>, day: u8, hour: Option<u16>, window: &mut Window, cx: &mut Context<Self>) {
        let c = id.as_ref().and_then(|id| self.data.classes.iter().find(|c| &c.id == id)).cloned();
        let c = c.unwrap_or_else(|| {
            let start = hour.unwrap_or(9) * 60;
            Class { day, start, end: start + 90, color: crate::theme::FOLDER_COLORS[(self.data.courses().len() + 5) % 7].into(), ..Default::default() }
        });
        self.set(&self.name, &c.course, cx); self.set(&self.kind, &c.kind, cx); self.set(&self.place, &c.place, cx);
        self.set(&self.start, &model::hm(c.start), cx); self.set(&self.end, &model::hm(c.end), cx);
        self.form = Some(Form::Class { id, day: c.day, color: c.color });
        self.error = None;
        window.focus(&self.name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    fn open_item(&mut self, id: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let today = now().0;
        let i = id.as_ref().and_then(|id| self.data.items.iter().find(|i| &i.id == id)).cloned().unwrap_or(Item { day: today + 7, ..Default::default() });
        self.set(&self.name, &i.title, cx); self.set(&self.course, &i.course, cx);
        self.set(&self.date, &if id.is_some() { let (y, m, d) = model::ymd(i.day); format!("{y}-{m:02}-{d:02}") } else { String::new() }, cx);
        self.set(&self.time, &i.time.map(model::hm).unwrap_or_default(), cx);
        self.form = Some(Form::Item { id, kind: i.kind, done: i.done });
        self.error = None;
        window.focus(&self.name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) { self.form = None; self.error = None; cx.notify(); }

    fn save_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.clone() else { return };
        let name = self.text(&self.name, cx);
        match form {
            Form::Class { id, day, color } => {
                if name.is_empty() { self.error = Some("Give the class a course name."); return cx.notify() }
                let (Some(start), Some(end)) = (model::parse_time(&self.text(&self.start, cx)), model::parse_time(&self.text(&self.end, cx))) else {
                    self.error = Some("Times look like 9:00, 14:30 or 2pm."); return cx.notify()
                };
                if end <= start { self.error = Some("The class has to end after it starts."); return cx.notify() }
                let c = Class { id: id.clone().unwrap_or_else(model::new_id), course: name, kind: self.text(&self.kind, cx), place: self.text(&self.place, cx), color, day, start, end };
                match id.and_then(|id| self.data.classes.iter_mut().find(|x| x.id == id)) { Some(x) => *x = c, None => self.data.classes.push(c) }
            }
            Form::Item { id, kind, done } => {
                if name.is_empty() { self.error = Some("Give it a title."); return cx.notify() }
                let Some(day) = model::parse_day(&self.text(&self.date, cx), now().0) else {
                    self.error = Some("Dates look like fri, tomorrow, 3.12 or 2026-12-03."); return cx.notify()
                };
                let t = self.text(&self.time, cx);
                let time = if t.is_empty() { None } else { match model::parse_time(&t) { Some(m) => Some(m), None => { self.error = Some("Times look like 9:00, 14:30 or 2pm."); return cx.notify() } } };
                let i = Item { id: id.clone().unwrap_or_else(model::new_id), title: name, course: self.text(&self.course, cx), kind, day, time, done };
                match id.and_then(|id| self.data.items.iter_mut().find(|x| x.id == id)) { Some(x) => *x = i, None => self.data.items.push(i) }
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
        let line = |dot: Hsla, head: String, sub: String| div().flex().items_start().gap(px(8.))
            .child(div().mt(px(5.)).size(px(8.)).flex_none().rounded_full().bg(dot))
            .child(div().min_w_0().flex().flex_col()
                .child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(head))
                .child(div().text_size(px(11.)).text_color(color::muted_foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(sub)));
        let mut lines = vec![];
        if let Some(c) = nn.now {
            lines.push(line(ink(&c.color), format!("Now · {}", c.course), join(&[&c.kind, &format!("until {}", model::hm(c.end)), &c.place])));
        } else if let Some((d, c)) = nn.next {
            let when = if d == day { model::until((day, min), (d, c.start)) } else { format!("{} {}", model::day_label(d), model::hm(c.start)) };
            lines.push(line(ink(&c.color), format!("Next · {}", c.course), join(&[&when, &c.place])));
        }
        if let Some(i) = self.data.upcoming(day, OVERDUE_DAYS).first() {
            let late = i.day < day;
            let dot = if late { color::destructive() } else { self.data.color_of(&i.course).map(ink).unwrap_or_else(color::muted_foreground) };
            lines.push(line(dot, format!("Due · {}", i.title), model::until((day, min), (i.day, i.time.unwrap_or(23 * 60 + 59)))));
        }
        if lines.is_empty() { return None }
        Some(div().flex().flex_col().gap(px(8.)).children(lines))
    }

    // ---- the page

    fn week_start(&self, today: Day) -> Day { today - model::weekday_of(today) as i64 + 7 * self.week }

    fn toolbar(&self, today: Day, cx: &mut Context<Self>) -> Div {
        let first = self.week_start(today);
        let (_, m1, d1) = model::ymd(first);
        let (y, m2, d2) = model::ymd(first + 6);
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let range = if m1 == m2 { format!("{d1} – {d2} {} {y}", MONTHS[m2 as usize - 1]) } else { format!("{d1} {} – {d2} {} {y}", MONTHS[m1 as usize - 1], MONTHS[m2 as usize - 1]) };
        div().flex_none().h(px(48.)).px(px(16.)).flex().items_center().gap(px(6.))
            .child(Button::ghost("sch-prev").icon("chevron-left").on_click(cx.listener(|p, _, _, cx| { p.week -= 1; cx.notify(); })))
            .child(Button::ghost("sch-next").icon("chevron-right").on_click(cx.listener(|p, _, _, cx| { p.week += 1; cx.notify(); })))
            .child(div().ml(px(4.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(color::foreground()).child(range))
            .when(self.week != 0, |d| d.child(Button::outline("sch-today").sized(ButtonSize::Sm).ml(px(8.)).label(tr("本周", "This week")).on_click(cx.listener(|p, _, _, cx| { p.week = 0; cx.notify(); }))))
            .child(div().flex_1())
            .child(Button::outline("sch-add-item").sized(ButtonSize::Sm).icon("plus").label(tr("作业 / 考试", "Assignment")).on_click(cx.listener(|p, _, window, cx| p.open_item(None, window, cx))))
            .child(Button::primary("sch-add-class").sized(ButtonSize::Sm).icon("plus").label(tr("课程", "Class")).on_click(cx.listener(|p, _, window, cx| p.open_class(None, now().2, None, window, cx))))
    }

    fn grid(&self, today: Day, minute: Minute, cx: &mut Context<Self>) -> Div {
        let first = self.week_start(today);
        let (lo, hi) = self.data.hours();
        let h = (hi - lo) as f32 * HOUR_H;
        let line = || color::border();
        // Day names with the date; under them, what is due that day.
        let head = div().flex_none().flex().border_b_1().border_color(line()).child(div().w(px(GUTTER_W)).flex_none())
            .children((0..7).map(|k| {
                let d = first + k;
                let is_today = d == today;
                let (_, _, dd) = model::ymd(d);
                let items = self.data.items_on(d);
                div().flex_1().min_w_0().px(px(4.)).pb(px(6.)).flex().flex_col().gap(px(3.))
                    .child(div().flex().items_baseline().gap(px(6.)).px(px(4.))
                        .child(div().text_size(px(11.)).text_color(color::muted_foreground()).child(model::WEEKDAYS[k as usize]))
                        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).when(is_today, |d| d.px(px(6.)).rounded_full().bg(color::destructive()).text_color(gpui::white()))
                            .when(!is_today, |d| d.text_color(color::foreground())).child(dd.to_string())))
                    .children(items.into_iter().map(|i| {
                        let c = self.data.color_of(&i.course).unwrap_or("gray").to_string();
                        let id = i.id.clone();
                        div().id(SharedString::from(format!("due-{}", i.id))).px(px(5.)).py(px(1.)).rounded(px(4.)).bg(wash(&c)).cursor_pointer()
                            .text_size(px(11.)).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis()
                            .when(i.done, |d| d.line_through().opacity(0.55))
                            .child(format!("{} {}", if i.kind == Kind::Exam { "★" } else { "•" }, i.title))
                            .on_click(cx.listener(move |p, _, window, cx| p.open_item(Some(id.clone()), window, cx)))
                    }))
            }));
        let gutter = div().w(px(GUTTER_W)).flex_none().relative().h(px(h)).children((lo..hi).map(|hr| {
            div().absolute().top(px((hr - lo) as f32 * HOUR_H - 7.)).right(px(8.)).text_size(px(10.5)).text_color(color::muted_foreground()).child(format!("{hr}:00"))
        }));
        let days = (0..7).map(|k| {
            let d = first + k;
            let wd = k as u8;
            let col = div().flex_1().min_w_0().relative().h(px(h)).border_l_1().border_color(line())
                .when(d == today, |d| d.bg(color::muted_half()))
                // An hour's cell: a click makes a class there.
                .children((lo..hi).map(|hr| {
                    div().id(("slot", (k as usize) * 32 + hr as usize)).absolute().left_0().right_0().top(px((hr - lo) as f32 * HOUR_H)).h(px(HOUR_H))
                        .border_t_1().border_color(line()).hover(|s| s.bg(color::accent_hover()))
                        .on_click(cx.listener(move |p, _, window, cx| p.open_class(None, wd, Some(hr), window, cx)))
                }))
                .children(self.data.classes_on(wd).into_iter().map(|c| {
                    let top = (c.start as f32 / 60. - lo as f32) * HOUR_H;
                    let height = ((c.end - c.start) as f32 / 60. * HOUR_H).max(20.);
                    let on = d == today && c.start <= minute && minute < c.end;
                    let id = c.id.clone();
                    div().id(SharedString::from(format!("class-{}", c.id))).occlude().absolute().top(px(top + 1.)).h(px(height - 2.)).left(px(3.)).right(px(3.))
                        .rounded(px(6.)).bg(wash(&c.color)).border_l(px(3.)).border_color(ink(&c.color)).px(px(6.)).py(px(3.)).overflow_hidden().cursor_pointer()
                        .when(on, |d| d.shadow(vec![BoxShadow { color: ink(&c.color).opacity(0.5), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.5), inset: false }]))
                        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(c.course.clone()))
                        .when(height > 34., |d| d.child(div().text_size(px(10.5)).text_color(color::muted_foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis()
                            .child(join(&[&format!("{}–{}", model::hm(c.start), model::hm(c.end)), &c.kind, &c.place]))))
                        .on_click(cx.listener(move |p, _, window, cx| p.open_class(Some(id.clone()), wd, None, window, cx)))
                }))
                // The time now.
                .when(d == today && minute >= lo * 60 && minute < hi * 60, |col| {
                    let y = (minute as f32 / 60. - lo as f32) * HOUR_H;
                    col.child(div().absolute().left_0().right_0().top(px(y - 1.)).h(px(2.)).bg(color::destructive()))
                        .child(div().absolute().left(px(-4.)).top(px(y - 4.)).size(px(8.)).rounded_full().bg(color::destructive()))
                });
            col
        });
        div().flex_1().min_w_0().flex().flex_col()
            .child(head)
            .child(div().id("sch-grid").flex_1().min_h_0().overflow_y_scroll().child(div().flex().pt(px(8.)).pb(px(16.)).child(gutter).children(days)))
    }

    fn panel(&self, today: Day, minute: Minute, cx: &mut Context<Self>) -> Div {
        let heading = |s: &'static str| div().mt(px(18.)).mb(px(8.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(color::muted_foreground()).child(s);
        let nn = self.data.now_next(today, minute);
        let card = |c: &Class, top: String, sub: String| div().p(px(12.)).rounded(px(10.)).bg(wash(&c.color)).border_l(px(3.)).border_color(ink(&c.color)).flex().flex_col().gap(px(2.))
            .child(div().text_size(px(11.)).text_color(color::muted_foreground()).child(top))
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(color::foreground()).child(c.course.clone()))
            .child(div().text_size(px(12.)).text_color(color::muted_foreground()).child(sub));
        let mut p = div().w(px(PANEL_W)).flex_none().h_full().border_l_1().border_color(color::border()).px(px(16.)).pb(px(24.)).flex().flex_col();
        if self.data.classes.is_empty() && self.data.items.is_empty() {
            return p.child(heading(tr("开始", "Get started")))
                .child(div().text_size(px(13.)).text_color(color::muted_foreground()).line_height(px(19.))
                    .child(tr("点网格里的一个时间加课程，或用上面的按钮。截止日期用“作业 / 考试”。", "Click an hour in the grid to add a class there, or use + Class. Deadlines and exams go under + Assignment.")));
        }
        p = p.child(heading(tr("现在", "Now")));
        p = match nn.now {
            Some(c) => p.child(card(c, format!("{} – {}", model::hm(c.start), model::hm(c.end)), join(&[&c.kind, &c.place, &format!("ends {}", model::until((today, minute), (today, c.end)))]))),
            None => p.child(div().text_size(px(13.)).text_color(color::muted_foreground()).child(tr("现在没有课。", "No class right now."))),
        };
        if let Some((d, c)) = nn.next {
            let when = if d == today { format!("{} · {}", model::hm(c.start), model::until((today, minute), (d, c.start))) } else { format!("{} · {}", model::day_label(d), model::hm(c.start)) };
            p = p.child(heading(tr("接下来", "Next"))).child(card(c, when, join(&[&c.kind, &c.place])));
        }
        // Today: the classes, then what is due today.
        let classes = self.data.classes_on(model::weekday_of(today));
        if !classes.is_empty() {
            p = p.child(heading(tr("今天", "Today"))).children(classes.into_iter().map(|c| {
                let over = c.end <= minute;
                div().flex().items_center().gap(px(8.)).py(px(4.)).when(over, |d| d.opacity(0.5))
                    .child(div().w(px(76.)).flex_none().text_size(px(12.)).text_color(color::muted_foreground()).child(format!("{}–{}", model::hm(c.start), model::hm(c.end))))
                    .child(div().size(px(8.)).flex_none().rounded_full().bg(ink(&c.color)))
                    .child(div().min_w_0().text_size(px(13.)).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(join(&[&c.course, &c.place])))
            }));
        }
        let due = self.data.upcoming(today, OVERDUE_DAYS);
        p = p.child(heading(tr("待完成", "Due")));
        if due.is_empty() { p = p.child(div().text_size(px(13.)).text_color(color::muted_foreground()).child(tr("没有待完成的作业。", "Nothing due. Nice."))); }
        p.children(due.into_iter().take(12).map(|i| {
            let late = i.day < today || (i.day == today && i.time.is_some_and(|t| t < minute));
            let c = self.data.color_of(&i.course).unwrap_or("gray").to_string();
            let (tick, open) = (i.id.clone(), i.id.clone());
            let when = format!("{}{} · {}", model::day_label(i.day), i.time.map(|t| format!(" {}", model::hm(t))).unwrap_or_default(), model::until((today, minute), (i.day, i.time.unwrap_or(23 * 60 + 59))));
            div().id(SharedString::from(format!("due-row-{}", i.id))).flex().items_start().gap(px(10.)).py(px(6.)).px(px(6.)).mx(px(-6.)).rounded(px(6.)).cursor_pointer()
                .hover(|s| s.bg(color::accent_hover()))
                .on_click(cx.listener(move |p, _, window, cx| p.open_item(Some(open.clone()), window, cx)))
                .child(div().id(SharedString::from(format!("due-tick-{}", i.id))).mt(px(1.)).occlude().child(Checkbox::new(i.done))
                    .on_click(cx.listener(move |p, _, _, cx| { cx.stop_propagation(); p.toggle_done(&tick, cx); })))
                .child(div().min_w_0().flex_1().flex().flex_col().gap(px(1.))
                    .child(div().flex().items_center().gap(px(6.))
                        .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(i.title.clone()))
                        .when(i.kind != Kind::Assignment, |d| d.child(design::Badge::new(i.kind.label()))))
                    .child(div().flex().items_center().gap(px(6.))
                        .when(!i.course.is_empty(), |d| d.child(div().size(px(7.)).flex_none().rounded_full().bg(ink(&c))).child(div().text_size(px(11.5)).text_color(color::muted_foreground()).child(i.course.clone())))
                        .child(div().text_size(px(11.5)).text_color(if late { color::destructive() } else { color::muted_foreground() }).child(when))))
        }))
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
                .child(div().flex().flex_col().gap(px(6.)).child(Label::new("Day"))
                    .child(Segmented::new().children((0..7u8).map(|k| Segment::new(("f-day", k as usize), model::WEEKDAYS[k as usize]).selected(k == *day)
                        .on_click(cx.listener(move |p, _, _, cx| { if let Some(Form::Class { day, .. }) = p.form.as_mut() { *day = k; } cx.notify(); }))))))
                .child(div().flex().gap(px(10.)).child(self.field("f-start", "Starts", &self.start, window, cx)).child(self.field("f-end", "Ends", &self.end, window, cx)))
                .child(self.field("f-place", "Room", &self.place, window, cx))
                .child(div().flex().flex_col().gap(px(6.)).child(Label::new("Colour"))
                    .child(div().flex().gap(px(4.)).children(crate::theme::FOLDER_COLORS.iter().enumerate().map(|(k, name)| {
                        let on = picked == name;
                        div().id(("f-color", k)).size(px(26.)).rounded_full().cursor_pointer().flex().items_center().justify_center()
                            .border(px(if on { 2. } else { 0. })).border_color(color::ring()).hover(|s| s.bg(color::accent()))
                            .child(div().size(px(16.)).rounded_full().bg(ink(name)))
                            .on_click(cx.listener(move |p, _, _, cx| { if let Some(Form::Class { color, .. }) = p.form.as_mut() { *color = (*name).into(); } cx.notify(); }))
                    })))),
            Form::Item { kind, done, .. } => {
                let courses = self.data.courses();
                div().flex().flex_col().gap(px(14.))
                    .child(div().flex().flex_col().gap(px(6.)).child(Label::new("What"))
                        .child(Segmented::new().children(Kind::ALL.iter().enumerate().map(|(k, x)| { let x = *x; Segment::new(("f-ikind", k), x.label()).selected(x == *kind)
                            .on_click(cx.listener(move |p, _, _, cx| { if let Some(Form::Item { kind, .. }) = p.form.as_mut() { *kind = x; } cx.notify(); })) }))))
                    .child(self.field("f-title", "Title", &self.name, window, cx))
                    .child(div().flex().flex_col().gap(px(6.)).child(self.field("f-icourse", "Course", &self.course, window, cx))
                        .when(!courses.is_empty(), |d| d.child(div().flex().flex_wrap().gap(px(4.)).children(courses.into_iter().enumerate().map(|(k, (name, c))| {
                            let n = name.clone();
                            div().id(("f-pick", k)).px(px(8.)).py(px(2.)).rounded_full().bg(wash(&c)).cursor_pointer().text_size(px(11.5)).text_color(color::foreground()).child(name)
                                .on_click(cx.listener(move |p, _, _, cx| { let f = p.course.clone(); f.update(cx, |f, cx| f.set_text(&n, cx)); cx.notify(); }))
                        })))))
                    .child(div().flex().gap(px(10.)).child(self.field("f-date", "Due", &self.date, window, cx)).child(self.field("f-time", "Time", &self.time, window, cx)))
                    .when(editing, |d| d.child(div().id("f-done").flex().items_center().gap(px(8.)).cursor_pointer().child(Checkbox::new(*done)).child(Label::new("Done"))
                        .on_click(cx.listener(|p, _, _, cx| { if let Some(Form::Item { done, .. }) = p.form.as_mut() { *done = !*done; } cx.notify(); }))))
            }
        };
        let title = match (&form, editing) { (Form::Class { .. }, false) => "New class", (Form::Class { .. }, true) => "Edit class", (Form::Item { .. }, false) => "New assignment or exam", (Form::Item { .. }, true) => "Edit" };
        let footer = div().w_full().flex().items_center().gap(px(8.))
            .when(editing, |d| d.child(Button::ghost("f-delete").sized(ButtonSize::Default).icon("bin").label("Delete").on_click(cx.listener(|p, _, _, cx| p.delete_form(cx)))))
            .child(div().flex_1())
            .child(Button::outline("f-cancel").label("Cancel").on_click(cx.listener(|p, _, _, cx| p.close_form(cx))))
            .child(Button::primary("f-save").label("Save").on_click(cx.listener(|p, _, _, cx| p.save_form(cx))));
        Some(design::scrim().id("sch-scrim").occlude()
            .on_key_down(cx.listener(|p, ev: &KeyDownEvent, _, cx| if ev.keystroke.key == "escape" { p.close_form(cx); cx.stop_propagation(); }))
            .child(design::Dialog::new("sch-form").w(px(460.)).title(title)
                .closable(cx.listener(|p, _, _, cx| p.close_form(cx)))
                .child(body)
                .when_some(self.error, |d, e| d.child(div().text_size(px(12.5)).text_color(color::destructive()).child(e)))
                .footer(footer))
            .into_any_element())
    }
}

/// The parts that are there, joined with " · ".
fn join(parts: &[&String]) -> String { parts.iter().filter(|s| !s.is_empty()).map(|s| s.as_str()).collect::<Vec<_>>().join(" · ") }

impl Render for SchedulePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (today, minute, _) = now();
        let form = self.form(window, cx);
        div().id("schedule").relative().size_full().flex().flex_col().font_family(crate::theme::ui_family()).text_color(color::foreground())
            .child(self.toolbar(today, cx))
            .child(div().flex_1().min_h_0().flex().child(self.grid(today, minute, cx)).child(div().id("sch-panel").h_full().overflow_y_scroll().child(self.panel(today, minute, cx))))
            .children(form)
    }
}
