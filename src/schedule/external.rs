//! 日历同步 on the schedule page (fork feature): the events of the calendars this Mac syncs (Google,
//! iCloud …, through `calendars`), drawn in the grid in their calendar's colour beside the classes;
//! the Calendars section that connects them and shows or hides each one; and the form that makes,
//! changes or deletes an event, which goes back to its calendar.

use super::calendars::{self, Access, Calendar, Draft, Event};
use super::model::{self, Day, Described, Link, Minute};
use super::{Block, SchedulePage, join, now};
use crate::design::{self, Button, ButtonSize, Checkbox, Label, color};
use gpui::{prelude::FluentBuilder, *};
use std::time::Duration;
use web_time::Instant;

/// How long fetched events count as fresh: Google's own sync reaches EventKit in the background.
const FRESH: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(super) struct Ext {
    pub access: Option<Access>,
    pub cals: Vec<Calendar>,
    pub events: Vec<Event>,
    /// The events of the four weeks around now, read when a note's 关联到课表 opens.
    pub around: Vec<Event>,
    /// The unix-seconds range `events` covers, and when it was read.
    pub range: Option<(i64, i64)>,
    pub read_at: Option<Instant>,
    pub form: Option<ExtForm>,
    /// 删除 asked once: the form shows its confirmation.
    pub deleting: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
pub(super) struct ExtForm {
    /// Editing this occurrence (id, start); None: a new event.
    pub old: Option<(String, i64)>,
    pub calendar: String,
    pub all_day: bool,
}

/// Unix seconds of a local day and minute.
pub(super) fn unix_of(day: Day, minute: Minute) -> i64 {
    let (y, m, d) = model::ymd(day);
    crate::time::local_to_unix(y, m, d, (minute / 60) as u32, (minute % 60) as u32)
}

fn ink_of(rgb: u32) -> Hsla { gpui::rgb(rgb).into() }
fn wash_of(rgb: u32) -> Hsla { ink_of(rgb).opacity(if crate::theme::dark() { 0.28 } else { 0.15 }) }

impl SchedulePage {
    pub(super) fn ext_access(&self) -> Access { self.ext.access.unwrap_or(Access::Unsupported) }

    /// Reads the calendars and the events around the days shown, when they are stale or the range
    /// moved (`force`: now).
    pub(super) fn ext_refresh(&mut self, force: bool) {
        let access = calendars::access();
        self.ext.access = Some(access);
        if access != Access::Full { self.ext.cals.clear(); self.ext.events.clear(); self.ext.range = None; return }
        let first = self.first_day();
        let range = (unix_of(first - 1, 0), unix_of(first + self.days as i64 + 1, 0));
        let stale = self.ext.read_at.is_none_or(|t| t.elapsed() > FRESH);
        if !force && !stale && self.ext.range == Some(range) { return }
        let (cals, events) = (calendars::calendars(), calendars::events(range.0, range.1));
        if cals.len() != self.ext.cals.len() || events.len() != self.ext.events.len() {
            log::info!("calendars: {} calendars ({} writable), {} events in the days shown", cals.len(), cals.iter().filter(|c| c.writable).count(), events.len());
        }
        self.ext.cals = cals;
        self.ext.events = events;
        self.ext.range = Some(range);
        self.ext.read_at = Some(Instant::now());
    }

    fn shown(&self, calendar: &str) -> bool { !self.data.hidden_calendars.iter().any(|h| h == calendar) }
    fn cal(&self, id: &str) -> Option<&Calendar> { self.ext.cals.iter().find(|c| c.id == id) }
    fn rgb_of(&self, calendar: &str) -> u32 { self.cal(calendar).map_or(0x8a8f98, |c| c.rgb) }

    /// The timed events on `d`, clipped to the day, as grid blocks.
    pub(super) fn ext_blocks(&self, d: Day) -> Vec<Block> {
        let (from, to) = (unix_of(d, 0), unix_of(d + 1, 0));
        self.ext.events.iter().filter(|e| !e.all_day && self.shown(&e.calendar) && e.start < to && e.end > from).map(|e| {
            let start = if e.start <= from { 0 } else { model::local(e.start as u64).1 };
            let end = if e.end >= to { 24 * 60 } else { model::local(e.end as u64).1.max(start + 15) };
            let rgb = self.rgb_of(&e.calendar);
            Block {
                link: Link::Event { id: e.id.clone(), start: e.start }, start, end: end.min(24 * 60), title: if e.title.is_empty() { "New event".into() } else { e.title.clone() },
                sub: join(&[&e.location, &self.cal(&e.calendar).map(|c| c.title.clone()).unwrap_or_default()]), accent: ink_of(rgb), fill: wash_of(rgb), done: false,
            }
        }).collect()
    }

    /// The all-day events covering `d`.
    pub(super) fn ext_all_day(&self, d: Day) -> Vec<&Event> {
        let (from, to) = (unix_of(d, 0), unix_of(d + 1, 0));
        self.ext.events.iter().filter(|e| e.all_day && self.shown(&e.calendar) && e.start < to && e.end > from).collect()
    }

    /// An all-day event's chip in the all-day row.
    pub(super) fn ext_chip(&self, e: &Event, cx: &mut Context<Self>) -> Stateful<Div> {
        let rgb = self.rgb_of(&e.calendar);
        let link = Link::Event { id: e.id.clone(), start: e.start };
        div().id(SharedString::from(format!("allday-ext-{}-{}", e.id, e.start))).h(px(20.)).px(px(6.)).rounded(px(5.)).bg(wash_of(rgb)).cursor_pointer()
            .flex().items_center().gap(px(5.)).overflow_hidden().hover(|s| s.opacity(0.85))
            .child(div().size(px(6.)).flex_none().rounded_full().bg(ink_of(rgb)))
            .child(div().min_w_0().text_size(px(11.5)).font_weight(FontWeight::MEDIUM).text_color(color::foreground()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(e.title.clone()))
            .on_click(cx.listener(move |p, _, _, cx| p.open_detail(link.clone(), cx)))
    }

    /// An event by its occurrence: from what was read, else from EventKit.
    pub(super) fn ext_event(&self, id: &str, start: i64) -> Option<Event> {
        self.ext.events.iter().chain(&self.ext.around).find(|e| e.id == id && e.start == start).cloned().or_else(|| calendars::event(id, start))
    }

    /// What a linked event is, as the chip and the details card say it, with its calendar's colour.
    pub(super) fn ext_describe(&self, id: &str, start: i64) -> Option<(Described, Hsla)> {
        let e = self.ext_event(id, start)?;
        let (day, minute, _) = model::local(e.start as u64);
        let cal = self.cal(&e.calendar).map(|c| c.title.clone()).unwrap_or_default();
        Some((Described { title: if e.title.is_empty() { "New event".into() } else { e.title.clone() }, sub: join(&[&cal, &model::day_label(day)]), color: String::new(), day, start: (!e.all_day).then_some(minute) },
            ink_of(self.rgb_of(&e.calendar))))
    }

    /// When an event is, as the details card says it.
    pub(super) fn ext_when(e: &Event) -> String {
        let (d0, m0, _) = model::local(e.start as u64);
        if e.all_day { return format!("{} · all day", model::day_label(d0)) }
        let (d1, m1, _) = model::local(e.end as u64);
        if d1 == d0 { format!("{} · {} – {}", model::day_label(d0), model::clock(m0), model::clock(m1)) }
        else { format!("{} {} – {} {}", model::day_label(d0), model::clock(m0), model::day_label(d1), model::clock(m1)) }
    }

    /// Reads the events from two weeks back to two weeks ahead, for 关联到课表.
    pub(super) fn read_around(&mut self, today: Day) {
        self.ext_refresh(false);
        self.ext.around = if self.ext_access() == Access::Full { calendars::events(unix_of(today - 14, 0), unix_of(today + 15, 0)) } else { vec![] };
    }

    /// The events of `around` a note could be linked to, each with how far it is from now in minutes.
    pub(super) fn ext_choices(&self) -> Vec<(i64, Link)> {
        let now = crate::time::now() as i64;
        self.ext.around.iter().filter(|e| self.shown(&e.calendar)).map(|e| {
            let on = e.start <= now && now < e.end;
            (if on { -1 } else { (e.start - now).abs() / 60 }, Link::Event { id: e.id.clone(), start: e.start })
        }).collect()
    }

    pub(super) fn ext_writable(&self, e: &Event) -> bool { self.cal(&e.calendar).is_some_and(|c| c.writable) }
    pub(super) fn ext_calendar_title(&self, e: &Event) -> String { self.cal(&e.calendar).map(|c| format!("{} · {}", c.title, c.account)).unwrap_or_default() }

    // ---- connecting

    /// 连接日历: macOS asks; once answered, the events come in.
    fn connect(&mut self, cx: &mut Context<Self>) {
        calendars::request();
        cx.spawn(async move |this, cx| {
            for _ in 0..600 {
                cx.background_executor().timer(Duration::from_millis(300)).await;
                if calendars::answered() || calendars::access() != Access::NotAsked {
                    let _ = this.update(cx, |p, cx| { calendars::reset(); p.ext_refresh(true); cx.notify(); });
                    return;
                }
            }
        }).detach();
    }

    fn toggle_calendar(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(i) = self.data.hidden_calendars.iter().position(|h| h == id) { self.data.hidden_calendars.remove(i); } else { self.data.hidden_calendars.push(id.to_string()); }
        self.commit(cx);
    }

    /// The side panel's Calendars section: connect, or the calendars by account with a switch each.
    pub(super) fn calendars_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        let heading = div().mt(px(22.)).mb(px(8.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(color::foreground()).child("Calendars");
        let muted = |s: &'static str| div().text_size(px(12.5)).line_height(px(18.)).text_color(color::muted_foreground()).child(s);
        let open = |url: &'static str| move |_: &ClickEvent, _: &mut Window, cx: &mut App| cx.open_url(url);
        const PRIVACY: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars";
        const ACCOUNTS: &str = "x-apple.systempreferences:com.apple.Internet-Accounts-Settings.extension";
        Some(match self.ext_access() {
            Access::Unsupported => return None,
            Access::NotAsked => div().flex().flex_col().child(heading)
                .child(div().p(px(14.)).rounded(px(12.)).bg(color::muted()).flex().flex_col().gap(px(10.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(color::foreground()).child("Google & Apple calendars"))
                    .child(muted("See your Google and iCloud events here beside your classes, and edit them. Changes sync back through your Mac."))
                    .child(Button::primary("cal-connect").sized(ButtonSize::Sm).icon("calendar").label("Connect calendars").on_click(cx.listener(|p, _, _, cx| p.connect(cx))))),
            Access::Denied => div().flex().flex_col().gap(px(8.)).child(heading)
                .child(muted("Calendar access is turned off for Moss. Turn it on in System Settings › Privacy & Security › Calendars, then come back."))
                .child(Button::outline("cal-privacy").sized(ButtonSize::Sm).label("Open Privacy Settings").on_click(open(PRIVACY))),
            Access::Full => {
                let mut accounts: Vec<String> = vec![];
                for c in &self.ext.cals { if !accounts.contains(&c.account) { accounts.push(c.account.clone()); } }
                let mut s = div().flex().flex_col().child(heading);
                if self.ext.cals.is_empty() { s = s.child(muted("No calendars on this Mac yet.")); }
                for (a, account) in accounts.iter().enumerate() {
                    s = s.child(div().mt(px(if a == 0 { 0. } else { 10. })).mb(px(2.)).text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(color::muted_foreground()).child(account.clone()));
                    for (k, c) in self.ext.cals.iter().enumerate().filter(|(_, c)| &c.account == account) {
                        let (on, id) = (self.shown(&c.id), c.id.clone());
                        s = s.child(div().id(("cal-row", k)).h(px(28.)).px(px(6.)).mx(px(-6.)).rounded(px(6.)).flex().items_center().gap(px(9.)).cursor_pointer().hover(|s| s.bg(color::accent_hover()))
                            .child(Checkbox::new(on)).child(div().size(px(9.)).flex_none().rounded_full().bg(ink_of(c.rgb)))
                            .child(div().min_w_0().flex_1().text_size(px(12.5)).text_color(if on { color::foreground() } else { color::muted_foreground() }).overflow_hidden().whitespace_nowrap().text_ellipsis().child(c.title.clone()))
                            .when(!c.writable, |d| d.child(div().text_size(px(10.5)).text_color(color::muted_foreground()).child("read-only")))
                            .on_click(cx.listener(move |p, _, _, cx| p.toggle_calendar(&id, cx))));
                    }
                }
                s.child(div().id("cal-add-account").mt(px(10.)).text_size(px(12.)).text_color(color::muted_foreground()).cursor_pointer().hover(|s| s.text_color(color::foreground()))
                    .child("Add a Google account…").on_click(open(ACCOUNTS)))
            }
        })
    }

    // ---- making and changing events

    /// Calendars a new event can go in: the shown, writable ones first.
    pub(super) fn writable_calendars(&self) -> Vec<&Calendar> {
        let mut v: Vec<&Calendar> = self.ext.cals.iter().filter(|c| c.writable).collect();
        v.sort_by_key(|c| !self.shown(&c.id));
        v
    }

    /// The event form: a new event on `day` (at `hour`), or `e` to change.
    pub(super) fn open_ext_form(&mut self, e: Option<Event>, day: Day, hour: Option<u16>, window: &mut Window, cx: &mut Context<Self>) {
        let (title, place, start, end, all_day, calendar, old) = match &e {
            Some(e) => (e.title.clone(), e.location.clone(), e.start, e.end, e.all_day, e.calendar.clone(), Some((e.id.clone(), e.start))),
            None => {
                let s = unix_of(day, hour.unwrap_or(9) * 60);
                (String::new(), String::new(), s, s + 3600, false, self.writable_calendars().first().map(|c| c.id.clone()).unwrap_or_default(), None)
            }
        };
        let (d0, m0, _) = model::local(start as u64);
        let (_, m1, _) = model::local(end as u64);
        let (y, m, d) = model::ymd(d0);
        self.set(&self.name, &title, cx); self.set(&self.place, &place, cx);
        self.set(&self.date, &format!("{y}-{m:02}-{d:02}"), cx);
        self.set(&self.time, &if all_day { String::new() } else { model::hm(m0) }, cx);
        self.set(&self.until, &if all_day { String::new() } else { model::hm(m1) }, cx);
        self.ext.form = Some(ExtForm { old, calendar, all_day });
        self.ext.deleting = false;
        self.ext.error = None;
        self.detail = None;
        self.add_menu = false;
        window.focus(&self.name.read(cx).focus_handle(cx), cx);
        cx.notify();
    }

    fn close_ext_form(&mut self, cx: &mut Context<Self>) { self.ext.form = None; self.ext.error = None; self.ext.deleting = false; cx.notify(); }

    pub(super) fn save_ext_form(&mut self, cx: &mut Context<Self>) {
        let Some(f) = self.ext.form.clone() else { return };
        let fail = |p: &mut Self, s: &str, cx: &mut Context<Self>| { p.ext.error = Some(s.to_string()); cx.notify(); };
        let title = self.text(&self.name, cx);
        if title.is_empty() { return fail(self, "Give the event a title.", cx) }
        if f.calendar.is_empty() { return fail(self, "Pick a calendar for it.", cx) }
        let Some(day) = model::parse_day(&self.text(&self.date, cx), now().0) else { return fail(self, "Dates look like fri, tomorrow, 3.12 or 2026-12-03.", cx) };
        let (start, end) = if f.all_day { (unix_of(day, 0), unix_of(day + 1, 0)) } else {
            let (Some(s), Some(e)) = (model::parse_time(&self.text(&self.time, cx)), model::parse_time(&self.text(&self.until, cx))) else { return fail(self, "Times look like 9:00, 14:30 or 2pm.", cx) };
            if e <= s { return fail(self, "It has to end after it starts.", cx) }
            (unix_of(day, s), unix_of(day, e))
        };
        let draft = Draft { calendar: f.calendar.clone(), title, location: self.text(&self.place, cx), start, end, all_day: f.all_day };
        match calendars::save(f.old.as_ref().map(|(id, s)| (id.as_str(), *s)), &draft) {
            Ok(()) => { self.ext.form = None; self.anchor = day; self.ext_refresh(true); cx.notify(); }
            Err(e) => fail(self, &e, cx),
        }
    }

    fn delete_ext(&mut self, cx: &mut Context<Self>) {
        let Some((id, start)) = self.ext.form.as_ref().and_then(|f| f.old.clone()) else { return };
        if !self.ext.deleting { self.ext.deleting = true; return cx.notify() }
        match calendars::delete(&id, start) {
            Ok(()) => { self.ext.form = None; self.ext.deleting = false; self.ext_refresh(true); cx.notify(); }
            Err(e) => { self.ext.error = Some(e); self.ext.deleting = false; cx.notify(); }
        }
    }

    pub(super) fn ext_form_view(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let f = self.ext.form.clone()?;
        let editing = f.old.is_some();
        let cals = self.writable_calendars();
        let account = self.cal(&f.calendar).map(|c| c.account.clone()).unwrap_or_default();
        let body = div().flex().flex_col().gap(px(14.))
            .child(self.field("x-title", "Title", &self.name, window, cx))
            .child(div().flex().gap(px(10.))
                .child(self.field("x-date", "Date", &self.date, window, cx))
                .when(!f.all_day, |d| d.child(self.field("x-start", "Starts", &self.time, window, cx)).child(self.field("x-end", "Ends", &self.until, window, cx))))
            .child(div().id("x-allday").flex().items_center().gap(px(8.)).cursor_pointer().child(Checkbox::new(f.all_day)).child(Label::new("All day"))
                .on_click(cx.listener(|p, _, _, cx| { if let Some(f) = p.ext.form.as_mut() { f.all_day = !f.all_day; } cx.notify(); })))
            .child(self.field("x-place", "Location", &self.place, window, cx))
            .child(div().flex().flex_col().gap(px(6.)).child(Label::new(if account.is_empty() { "Calendar".to_string() } else { format!("Calendar · {account}") }))
                .child(div().flex().flex_wrap().gap(px(6.)).children(cals.into_iter().enumerate().map(|(k, c)| {
                    let (on, id) = (c.id == f.calendar, c.id.clone());
                    div().id(("x-cal", k)).h(px(28.)).px(px(10.)).rounded_full().flex().items_center().gap(px(6.)).cursor_pointer()
                        .bg(if on { wash_of(c.rgb) } else { color::muted() }).border_1().border_color(if on { ink_of(c.rgb) } else { gpui::transparent_black() })
                        .child(div().size(px(8.)).rounded_full().bg(ink_of(c.rgb)))
                        .child(div().text_size(px(12.)).font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }).text_color(color::foreground()).child(c.title.clone()))
                        .on_click(cx.listener(move |p, _, _, cx| { if let Some(f) = p.ext.form.as_mut() { f.calendar = id.clone(); } cx.notify(); }))
                }))));
        let footer = div().w_full().flex().items_center().gap(px(8.))
            .when(editing, |d| d.child(if self.ext.deleting {
                Button::destructive("x-delete").label("Delete this event").on_click(cx.listener(|p, _, _, cx| p.delete_ext(cx)))
            } else {
                Button::ghost("x-delete").sized(ButtonSize::Default).icon("bin").label("Delete").on_click(cx.listener(|p, _, _, cx| p.delete_ext(cx)))
            }))
            .child(div().flex_1())
            .child(Button::outline("x-cancel").label("Cancel").on_click(cx.listener(|p, _, _, cx| p.close_ext_form(cx))))
            .child(Button::primary("x-save").label("Save").on_click(cx.listener(|p, _, _, cx| p.save_ext_form(cx))));
        Some(design::scrim().id("x-scrim").occlude()
            .on_key_down(cx.listener(|p, ev: &KeyDownEvent, _, cx| if ev.keystroke.key == "escape" { p.close_ext_form(cx); cx.stop_propagation(); }))
            .child(design::Dialog::new("x-form").w(px(480.)).title(if editing { "Edit event" } else { "New event" })
                .when(self.ext.deleting, |d| d.description("This deletes it from the calendar on every device. A repeating event loses only this one."))
                .closable(cx.listener(|p, _, _, cx| p.close_ext_form(cx)))
                .child(body)
                .when_some(self.ext.error.clone(), |d, e| d.child(div().text_size(px(12.5)).text_color(color::destructive()).child(e)))
                .footer(footer))
            .into_any_element())
    }
}
