//! 课表 (fork feature): what the schedule holds and the questions it answers (what is on now, what
//! is next, what is due), apart from GPUI so it can be tested alone. Kept per library beside
//! `preferences.json`, as `schedule.json`.

use serde::{Deserialize, Serialize};

/// Minutes since midnight.
pub type Minute = u16;
/// Days since 1970-01-01, local.
pub type Day = i64;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    /// Classes that repeat every week.
    pub classes: Vec<Class>,
    /// Assignments, exams and one-off events, each on one day.
    pub items: Vec<Item>,
    /// Calendars of this Mac (Google, iCloud …) left out of the schedule, by EventKit id.
    pub hidden_calendars: Vec<String>,
}

/// One weekly session of a course: "Calculus, Lecture, Mon 9:00–10:30, Room B12".
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Class {
    pub id: String,
    pub course: String,
    /// Lecture, Lab, Tutorial … (free text, may be empty).
    pub kind: String,
    pub place: String,
    /// A palette name (`theme::FOLDER_COLORS`).
    pub color: String,
    /// 0 = Monday … 6 = Sunday.
    pub day: u8,
    pub start: Minute,
    pub end: Minute,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind { #[default] Assignment, Exam, Event }

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Assignment, Kind::Exam, Kind::Event];
    pub fn label(self) -> &'static str { match self { Kind::Assignment => "Assignment", Kind::Exam => "Exam", Kind::Event => "Event" } }
}

/// Something on one day: an assignment due, an exam, an event.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    pub id: String,
    pub title: String,
    /// The course it belongs to (a `Class::course`), or empty.
    pub course: String,
    pub kind: Kind,
    pub day: Day,
    /// When on that day (due time, exam start); None: the whole day.
    pub time: Option<Minute>,
    /// When it ends (exams, events); None: a moment (a deadline) or the whole day.
    pub end: Option<Minute>,
    pub done: bool,
}

/// The local day, minute and weekday of unix seconds.
pub fn local(secs: u64) -> (Day, Minute, u8) {
    let (y, m, d, h, mi) = crate::time::civil(secs);
    (crate::time::days_from_civil(y, m, d), (h * 60 + mi) as Minute, crate::time::weekday(y, m, d) as u8)
}

pub fn weekday_of(day: Day) -> u8 { (day + 3).rem_euclid(7) as u8 }

/// (y, m, d) of a day number.
pub fn ymd(day: Day) -> (i64, u32, u32) {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "9:05", "14:30": what form fields hold (24-hour, read back by `parse_time`).
pub fn hm(m: Minute) -> String { format!("{}:{:02}", m / 60, m % 60) }

/// Whether this Mac's region reads the clock in 12 hours (en-US and its neighbours).
pub fn twelve_hour() -> bool {
    use std::sync::OnceLock;
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| sys_locale::get_locale().is_some_and(|l| ["en-US", "en-CA", "en-AU", "en-NZ", "en-PH", "en-IN"].iter().any(|p| l.starts_with(p)) || l == "en"))
}

/// A time as the region reads it: "9:05 AM" / "09:05".
pub fn clock(m: Minute) -> String {
    let (h, mi) = (m / 60, m % 60);
    if twelve_hour() { format!("{}:{mi:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" }) } else { format!("{h:02}:{mi:02}") }
}

/// An hour's label in the grid's margin: "9 AM", "Noon" / "09:00".
pub fn hour_label(h: u16) -> String {
    if !twelve_hour() { return format!("{h:02}:00") }
    match h { 0 | 24 => "12 AM".into(), 12 => "Noon".into(), h if h < 12 => format!("{h} AM"), h => format!("{} PM", h - 12) }
}

pub const MONTHS_LONG: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// Side by side: for blocks (start, end) on one day, each one's column and how many columns its
/// overlapping group takes, as a calendar lays out events that clash.
pub fn lay_out(blocks: &[(Minute, Minute)]) -> Vec<(usize, usize)> {
    let mut order: Vec<usize> = (0..blocks.len()).collect();
    order.sort_by_key(|&i| (blocks[i].0, std::cmp::Reverse(blocks[i].1)));
    let mut out = vec![(0, 1); blocks.len()];
    let (mut group, mut cols, mut group_end): (Vec<usize>, Vec<Minute>, Minute) = (vec![], vec![], 0);
    let close = |group: &mut Vec<usize>, cols: &mut Vec<Minute>, out: &mut Vec<(usize, usize)>| {
        for &i in group.iter() { out[i].1 = cols.len().max(1); }
        group.clear(); cols.clear();
    };
    for i in order {
        let (s, e) = blocks[i];
        // A block starting after everything so far has ended begins a new group.
        if !group.is_empty() && s >= group_end { close(&mut group, &mut cols, &mut out); group_end = 0; }
        let col = match cols.iter().position(|&end| end <= s) { Some(c) => { cols[c] = e; c } None => { cols.push(e); cols.len() - 1 } };
        out[i].0 = col;
        group.push(i);
        group_end = group_end.max(e);
    }
    close(&mut group, &mut cols, &mut out);
    out
}

/// A day as a date field shows it: "Thu 26 Nov", with the year when it is not this one.
pub fn day_input(day: Day, today: Day) -> String {
    let (y, _, _) = ymd(day);
    if y == ymd(today).0 { day_label(day) } else { format!("{} {y}", day_label(day)) }
}

/// "Mon 24 Nov".
pub fn day_label(day: Day) -> String { let (_, m, d) = ymd(day); format!("{} {} {}", WEEKDAYS[weekday_of(day) as usize], d, MONTHS[m as usize - 1]) }

/// "9", "9:30", "09:30", "930", "2pm", "2:15 pm" → minutes; None for anything else.
pub fn parse_time(s: &str) -> Option<Minute> {
    let s = s.trim().to_ascii_lowercase();
    let (s, pm, am) = if let Some(r) = s.strip_suffix("pm") { (r.trim().to_string(), true, false) } else if let Some(r) = s.strip_suffix("am") { (r.trim().to_string(), false, true) } else { (s, false, false) };
    let (h, m) = match s.split_once([':', '.']) {
        Some((h, m)) => (h.parse::<u16>().ok()?, m.parse::<u16>().ok()?),
        None if s.len() >= 3 && s.bytes().all(|b| b.is_ascii_digit()) => { let (h, m) = s.split_at(s.len() - 2); (h.parse().ok()?, m.parse().ok()?) }
        None => (s.parse::<u16>().ok()?, 0),
    };
    if m > 59 || h > 23 || ((pm || am) && !(1..=12).contains(&h)) { return None }
    let h = match (pm, am) { (true, _) if h < 12 => h + 12, (_, true) if h == 12 => 0, _ => h };
    Some(h * 60 + m)
}

/// "2026-12-03", "3.12.2026", "3/12" (this year), "today", "tomorrow", a weekday ("fri": the next
/// one, today counting) → a day; None for anything else.
pub fn parse_day(s: &str, today: Day) -> Option<Day> {
    let s = s.trim().to_ascii_lowercase();
    match s.as_str() {
        "today" => return Some(today),
        "tomorrow" => return Some(today + 1),
        _ => {}
    }
    if let Some(w) = WEEKDAYS.iter().position(|w| s.len() >= 3 && w.to_ascii_lowercase().starts_with(&s[..3]) && s.chars().all(|c| c.is_ascii_alphabetic())) {
        return Some(today + (w as i64 - weekday_of(today) as i64).rem_euclid(7));
    }
    // "Thu 26 Nov", "26 Nov", "26 November 2026", "Nov 26": what `day_input` writes, and what people type.
    let words: Vec<&str> = s.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()).collect();
    let month_of = |w: &str| (w.len() >= 3).then(|| MONTHS.iter().position(|m| m.to_ascii_lowercase() == w[..3])).flatten().map(|i| i as u32 + 1);
    if let Some(mi) = words.iter().position(|w| w.chars().all(|c| c.is_ascii_alphabetic()) && month_of(w).is_some()) {
        let m = month_of(words[mi])?;
        let nums: Vec<i64> = words.iter().filter_map(|w| w.parse::<i64>().ok()).collect();
        let (d, y) = match nums.as_slice() { [d] => (*d, ymd(today).0), [d, y] if *y > 31 => (*d, *y), [y, d] if *y > 31 => (*d, *y), _ => return None };
        return ((1..=31).contains(&d) && d as u32 <= crate::time::days_in_month(y, m)).then(|| crate::time::days_from_civil(y, m, d as u32));
    }
    let valid = |y: i64, m: u32, d: u32| ((1..=12).contains(&m) && d >= 1 && d <= crate::time::days_in_month(y, m)).then(|| crate::time::days_from_civil(y, m, d));
    let parts: Vec<&str> = s.split(['-', '.', '/']).filter(|p| !p.is_empty()).collect();
    let n = |p: &str| p.parse::<i64>().ok();
    match parts.as_slice() {
        [y, m, d] if y.len() == 4 => valid(n(y)?, n(m)? as u32, n(d)? as u32),
        [d, m, y] => valid(n(y)?, n(m)? as u32, n(d)? as u32),
        [d, m] => valid(ymd(today).0, n(m)? as u32, n(d)? as u32),
        _ => None,
    }
}

/// "in 25 min", "in 3 h", "tomorrow", "in 4 days", "2 days ago".
pub fn until(from: (Day, Minute), to: (Day, Minute)) -> String {
    let mins = (to.0 - from.0) * 1440 + to.1 as i64 - from.1 as i64;
    let days = to.0 - from.0;
    match () {
        _ if mins < 0 && days < 0 => if days == -1 { "yesterday".into() } else { format!("{} days ago", -days) },
        _ if mins < 0 => "earlier today".into(),
        _ if mins < 60 => format!("in {mins} min"),
        _ if days == 0 => { let h = (mins + 30) / 60; format!("in {h} h") }
        _ if days == 1 => "tomorrow".into(),
        _ => format!("in {days} days"),
    }
}

/// What a note is linked to (`Meta::event`): one lecture of a class, or one item.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    Class { id: String, day: Day },
    Item { id: String },
    /// One occurrence of an event in a calendar of this Mac: its EventKit id and start (unix seconds).
    Event { id: String, start: i64 },
}

impl Link {
    pub fn key(&self) -> String {
        match self { Link::Class { id, day } => format!("class:{id}:{day}"), Link::Item { id } => format!("item:{id}"), Link::Event { id, start } => format!("event:{start}:{id}") }
    }
    pub fn parse(s: &str) -> Option<Link> {
        if let Some(r) = s.strip_prefix("item:") { return (!r.is_empty()).then(|| Link::Item { id: r.into() }) }
        // An EventKit id can hold colons: the start comes first.
        if let Some(r) = s.strip_prefix("event:") {
            let (start, id) = r.split_once(':')?;
            let start = start.parse().ok()?;
            return (!id.is_empty()).then(|| Link::Event { id: id.into(), start });
        }
        let r = s.strip_prefix("class:")?;
        let (id, day) = r.rsplit_once(':')?;
        Some(Link::Class { id: id.into(), day: day.parse().ok()? })
    }
}

/// What a link points at, as people read it.
#[derive(Clone, Debug, PartialEq)]
pub struct Described { pub title: String, pub sub: String, pub color: String, pub day: Day, pub start: Option<Minute> }

/// What `Schedule::now_next` finds.
#[derive(Debug, PartialEq)]
pub struct NowNext<'a> {
    /// The class going on now.
    pub now: Option<&'a Class>,
    /// The next class to start, and on which day.
    pub next: Option<(Day, &'a Class)>,
}

impl Schedule {
    /// Classes on a weekday, by start time.
    pub fn classes_on(&self, weekday: u8) -> Vec<&Class> {
        let mut v: Vec<&Class> = self.classes.iter().filter(|c| c.day == weekday).collect();
        v.sort_by_key(|c| (c.start, c.end));
        v
    }

    /// Items on a day, by time (whole-day ones first).
    pub fn items_on(&self, day: Day) -> Vec<&Item> {
        let mut v: Vec<&Item> = self.items.iter().filter(|i| i.day == day).collect();
        v.sort_by_key(|i| (i.time, i.title.clone()));
        v
    }

    /// The class on now, and the next one to start within a week.
    pub fn now_next(&self, day: Day, minute: Minute) -> NowNext<'_> {
        let wd = weekday_of(day);
        let now = self.classes_on(wd).into_iter().find(|c| c.start <= minute && minute < c.end);
        let next = (0..8).find_map(|k| {
            let d = day + k;
            self.classes_on(weekday_of(d)).into_iter().find(|c| k > 0 || c.start > minute).map(|c| (d, c))
        });
        NowNext { now, next }
    }

    /// Items not done yet, from `day` - `back` on, soonest first; overdue ones come first.
    pub fn upcoming(&self, day: Day, back: i64) -> Vec<&Item> {
        let mut v: Vec<&Item> = self.items.iter().filter(|i| !i.done && i.day >= day - back).collect();
        v.sort_by_key(|i| (i.day, i.time.unwrap_or(24 * 60), i.title.clone()));
        v
    }

    /// The courses named so far, each with the colour its first class has.
    pub fn courses(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = vec![];
        for c in &self.classes { if !c.course.is_empty() && !out.iter().any(|(n, _)| *n == c.course) { out.push((c.course.clone(), c.color.clone())); } }
        out
    }

    /// What `link` points at; None once that class or item is gone.
    pub fn describe(&self, link: &Link) -> Option<Described> {
        match link {
            Link::Class { id, day } => self.classes.iter().find(|c| &c.id == id).map(|c| Described {
                title: c.course.clone(), sub: [c.kind.as_str(), &day_label(*day)].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" · "),
                color: c.color.clone(), day: *day, start: Some(c.start),
            }),
            Link::Item { id } => self.items.iter().find(|i| &i.id == id).map(|i| Described {
                title: i.title.clone(), sub: [i.kind.label(), &day_label(i.day)].join(" · "),
                color: self.color_of(&i.course).unwrap_or("gray").into(), day: i.day, start: i.time,
            }),
            // The page knows those (`SchedulePage::describe`), from EventKit.
            Link::Event { .. } => None,
        }
    }

    /// What a note could be linked to, around `today`: the lectures of the past and next `span` days
    /// (soonest to now first), then the items not yet done.
    pub fn link_choices(&self, today: Day, minute: Minute, span: i64) -> Vec<Link> {
        let mut lectures: Vec<(i64, Link)> = vec![];
        for d in today - span..=today + span {
            for c in self.classes_on(weekday_of(d)) {
                // The lecture on now comes first of all.
                let on = d == today && c.start <= minute && minute < c.end;
                let distance = if on { -1 } else { ((d - today) * 1440 + c.start as i64 - minute as i64).abs() };
                lectures.push((distance, Link::Class { id: c.id.clone(), day: d }));
            }
        }
        lectures.sort_by_key(|(d, _)| *d);
        lectures.into_iter().map(|(_, l)| l).chain(self.upcoming(today, span).into_iter().map(|i| Link::Item { id: i.id.clone() })).collect()
    }

    /// A course's colour (its first class's), if it has classes.
    pub fn color_of(&self, course: &str) -> Option<&str> { self.classes.iter().find(|c| c.course == course).map(|c| c.color.as_str()) }
}

// ---- on disk

fn path() -> std::path::PathBuf { crate::store::Store::settings_dir().join("schedule.json") }

pub fn load() -> Schedule {
    fdl_core::vfs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save(s: &Schedule) -> anyhow::Result<()> {
    use std::io::Write;
    let root = crate::store::Store::settings_dir();
    fdl_core::vfs::create_dir_all(&root)?;
    let tmp = root.join("schedule.json.tmp");
    let mut f = fdl_core::vfs::File::create(&tmp)?;
    f.write_all(&serde_json::to_vec_pretty(s)?)?;
    f.sync_all()?;
    fdl_core::vfs::rename(tmp, path())?;
    Ok(())
}

pub fn new_id() -> String { uuid::Uuid::new_v4().to_string() }

#[cfg(test)]
mod tests {
    use super::*;

    fn class(course: &str, day: u8, start: &str, end: &str) -> Class {
        Class { id: new_id(), course: course.into(), day, start: parse_time(start).unwrap(), end: parse_time(end).unwrap(), ..Default::default() }
    }

    #[test]
    fn times_parse_as_people_type_them() {
        assert_eq!(parse_time("9"), Some(540));
        assert_eq!(parse_time("9:30"), Some(570));
        assert_eq!(parse_time("09.30"), Some(570));
        assert_eq!(parse_time("1430"), Some(870));
        assert_eq!(parse_time("2pm"), Some(840));
        assert_eq!(parse_time("12 am"), Some(0));
        assert_eq!(parse_time("12:15pm"), Some(735));
        assert_eq!(parse_time("25"), None);
        assert_eq!(parse_time("9:75"), None);
        assert_eq!(parse_time("13pm"), None);
        assert_eq!(hm(570), "9:30");
    }

    #[test]
    fn days_parse_absolute_and_relative() {
        let today = crate::time::days_from_civil(2026, 11, 26); // a Thursday
        assert_eq!(weekday_of(today), 3);
        assert_eq!(parse_day("2026-12-03", today), Some(today + 7));
        assert_eq!(parse_day("3.12.2026", today), Some(today + 7));
        assert_eq!(parse_day("3/12", today), Some(today + 7));
        assert_eq!(parse_day("tomorrow", today), Some(today + 1));
        assert_eq!(parse_day("thu", today), Some(today));
        assert_eq!(parse_day("Friday", today), Some(today + 1));
        assert_eq!(parse_day("mon", today), Some(today + 4));
        assert_eq!(parse_day("31.2.2026", today), None);
        // Words, as the date field shows them.
        assert_eq!(parse_day("Thu 26 Nov", today), Some(today));
        assert_eq!(parse_day("3 Dec", today), Some(today + 7));
        assert_eq!(parse_day("december 3, 2026", today), Some(today + 7));
        assert_eq!(parse_day("Nov 30", today), Some(today + 4));
        assert_eq!(parse_day("Fri 5 Feb 2027", today), Some(crate::time::days_from_civil(2027, 2, 5)));
        assert_eq!(parse_day("30 Feb", today), None);
        assert_eq!(parse_day(&day_input(today + 7, today), today), Some(today + 7));
        assert_eq!(day_input(crate::time::days_from_civil(2027, 2, 5), today), "Fri 5 Feb 2027");
        assert_eq!(parse_day("soon", today), None);
        assert_eq!(day_label(today), "Thu 26 Nov");
        assert_eq!(ymd(today), (2026, 11, 26));
    }

    #[test]
    fn now_and_next_follow_the_week() {
        let today = crate::time::days_from_civil(2026, 11, 26); // Thursday = 3
        let s = Schedule { classes: vec![class("Calculus", 3, "9", "10:30"), class("Physics", 3, "14", "16"), class("History", 0, "10", "11")], items: vec![], ..Default::default() };
        let at = |m: &str| s.now_next(today, parse_time(m).unwrap());
        let r = at("9:15");
        assert_eq!(r.now.map(|c| c.course.as_str()), Some("Calculus"));
        assert_eq!(r.next.map(|(d, c)| (d, c.course.as_str())), Some((today, "Physics")));
        let r = at("12:00");
        assert_eq!(r.now, None);
        assert_eq!(r.next.map(|(_, c)| c.course.as_str()), Some("Physics"));
        // After the last class: next Monday's.
        let r = at("17:00");
        assert_eq!(r.next.map(|(d, c)| (d, c.course.as_str())), Some((today + 4, "History")));
        // Next week's Thursday when only Thursday classes exist and they are over.
        let s = Schedule { classes: vec![class("Calculus", 3, "9", "10")], items: vec![], ..Default::default() };
        assert_eq!(s.now_next(today, 700).next.map(|(d, _)| d), Some(today + 7));
        assert_eq!(Schedule::default().now_next(today, 0), NowNext { now: None, next: None });
    }

    #[test]
    fn upcoming_skips_done_and_old() {
        let today = 20_000;
        let item = |t: &str, day: Day, done: bool| Item { id: new_id(), title: t.into(), day, done, ..Default::default() };
        let s = Schedule { classes: vec![], items: vec![item("old", today - 30, false), item("late", today - 2, false), item("done", today + 1, true), item("essay", today + 3, false), item("quiz", today, false)], ..Default::default() };
        let v: Vec<&str> = s.upcoming(today, 14).iter().map(|i| i.title.as_str()).collect();
        assert_eq!(v, ["late", "quiz", "essay"]);
    }

    #[test]
    fn clashing_blocks_sit_side_by_side() {
        // 9–10 and 9:30–11 clash; 11–12 stands alone; 13–15, 13–14 and 14–15 make two columns.
        let b = [(540, 600), (570, 660), (660, 720), (780, 900), (780, 840), (840, 900)];
        let l = lay_out(&b);
        assert_eq!(l[0], (0, 2));
        assert_eq!(l[1], (1, 2));
        assert_eq!(l[2], (0, 1));
        assert_eq!(l[3], (0, 2));
        assert_eq!(l[4], (1, 2));
        assert_eq!(l[5], (1, 2));
        assert!(lay_out(&[]).is_empty());
    }

    #[test]
    fn links_round_trip_and_describe() {
        let today = crate::time::days_from_civil(2026, 11, 26);
        let c = class("Calculus", 3, "9", "10:30");
        let item = Item { id: "e1".into(), title: "Essay".into(), course: "Calculus".into(), day: today + 2, ..Default::default() };
        let s = Schedule { classes: vec![Class { kind: "Lecture".into(), color: "blue".into(), ..c.clone() }], items: vec![item], ..Default::default() };
        let l = Link::Class { id: c.id.clone(), day: today };
        assert_eq!(Link::parse(&l.key()), Some(l.clone()));
        assert_eq!(Link::parse("item:e1"), Some(Link::Item { id: "e1".into() }));
        assert_eq!(Link::parse("class:x"), None);
        let ev = Link::Event { id: "AB:CD:12".into(), start: 1_795_000_000 };
        assert_eq!(Link::parse(&ev.key()), Some(ev));
        assert_eq!(Link::parse("event:soon:x"), None);
        assert_eq!(Link::parse("nonsense"), None);
        let d = s.describe(&l).unwrap();
        assert_eq!((d.title.as_str(), d.sub.as_str(), d.color.as_str()), ("Calculus", "Lecture · Thu 26 Nov", "blue"));
        assert_eq!(s.describe(&Link::Item { id: "e1".into() }).unwrap().sub, "Assignment · Sat 28 Nov");
        assert_eq!(s.describe(&Link::Item { id: "gone".into() }), None);
        // The lecture going on now comes first (before one about to start), then the nearest others, then what is due.
        let soon = Schedule { classes: vec![s.classes[0].clone(), Class { start: 600, end: 660, ..class("Physics", 3, "10", "11") }], items: vec![], ..Default::default() };
        assert_eq!(soon.link_choices(today, 9 * 60 + 58, 0).first(), Some(&l));
        let choices = s.link_choices(today, 9 * 60 + 30, 7);
        assert_eq!(choices.first(), Some(&l));
        assert_eq!(choices.iter().filter(|c| matches!(c, Link::Class { .. })).count(), 3); // Thu 19, 26, 3 Dec
        assert_eq!(choices.last(), Some(&Link::Item { id: "e1".into() }));
    }

    #[test]
    fn until_reads_naturally() {
        assert_eq!(until((10, 600), (10, 625)), "in 25 min");
        assert_eq!(until((10, 600), (10, 790)), "in 3 h");
        assert_eq!(until((10, 600), (11, 0)), "tomorrow");
        assert_eq!(until((10, 600), (14, 0)), "in 4 days");
        assert_eq!(until((10, 600), (8, 0)), "2 days ago");
    }

    #[test]
    fn round_trips_through_json() {
        let s = Schedule { classes: vec![class("Calculus", 0, "9", "10")], items: vec![Item { id: "a".into(), title: "Essay".into(), kind: Kind::Exam, day: 5, time: Some(600), ..Default::default() }], ..Default::default() };
        let back: Schedule = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // Older or hand-written files with fields missing still read.
        let partial: Schedule = serde_json::from_str(r#"{"classes":[{"course":"X","day":2}]}"#).unwrap();
        assert_eq!(partial.classes[0].course, "X");
    }
}
