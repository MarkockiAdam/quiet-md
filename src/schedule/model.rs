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

/// "9:05", "14:30".
pub fn hm(m: Minute) -> String { format!("{}:{:02}", m / 60, m % 60) }

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

    /// A course's colour (its first class's), if it has classes.
    pub fn color_of(&self, course: &str) -> Option<&str> { self.classes.iter().find(|c| c.course == course).map(|c| c.color.as_str()) }

    /// The hours the week grid shows: 8–18 at least, widened to every class.
    pub fn hours(&self) -> (u16, u16) {
        let lo = self.classes.iter().map(|c| c.start / 60).min().unwrap_or(8).min(8);
        let hi = self.classes.iter().map(|c| c.end.div_ceil(60)).max().unwrap_or(18).max(18);
        (lo, hi.min(24))
    }
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
        assert_eq!(parse_day("soon", today), None);
        assert_eq!(day_label(today), "Thu 26 Nov");
        assert_eq!(ymd(today), (2026, 11, 26));
    }

    #[test]
    fn now_and_next_follow_the_week() {
        let today = crate::time::days_from_civil(2026, 11, 26); // Thursday = 3
        let s = Schedule { classes: vec![class("Calculus", 3, "9", "10:30"), class("Physics", 3, "14", "16"), class("History", 0, "10", "11")], items: vec![] };
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
        let s = Schedule { classes: vec![class("Calculus", 3, "9", "10")], items: vec![] };
        assert_eq!(s.now_next(today, 700).next.map(|(d, _)| d), Some(today + 7));
        assert_eq!(Schedule::default().now_next(today, 0), NowNext { now: None, next: None });
    }

    #[test]
    fn upcoming_skips_done_and_old() {
        let today = 20_000;
        let item = |t: &str, day: Day, done: bool| Item { id: new_id(), title: t.into(), day, done, ..Default::default() };
        let s = Schedule { classes: vec![], items: vec![item("old", today - 30, false), item("late", today - 2, false), item("done", today + 1, true), item("essay", today + 3, false), item("quiz", today, false)] };
        let v: Vec<&str> = s.upcoming(today, 14).iter().map(|i| i.title.as_str()).collect();
        assert_eq!(v, ["late", "quiz", "essay"]);
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
        let s = Schedule { classes: vec![class("Calculus", 0, "9", "10")], items: vec![Item { id: "a".into(), title: "Essay".into(), kind: Kind::Exam, day: 5, time: Some(600), ..Default::default() }] };
        let back: Schedule = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // Older or hand-written files with fields missing still read.
        let partial: Schedule = serde_json::from_str(r#"{"classes":[{"course":"X","day":2}]}"#).unwrap();
        assert_eq!(partial.classes[0].course, "X");
    }
}
