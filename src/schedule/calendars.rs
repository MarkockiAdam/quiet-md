//! 日历同步 (fork feature): the calendars this Mac already syncs (iCloud, Google, Exchange …, set up
//! in System Settings › Internet Accounts), read and written through EventKit, so an edit here goes
//! back to Google or iCloud the way one in Calendar does. Nothing is copied into the library: the
//! schedule asks EventKit for the days it shows. Other systems have no EventKit; there `access()`
//! says `Unsupported` and the page shows nothing of it.

/// What macOS lets this app do with calendars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access { Unsupported, NotAsked, Denied, Full }

/// One calendar: "Work" in jane@gmail.com, red, writable.
#[derive(Clone, Debug, PartialEq)]
pub struct Calendar { pub id: String, pub title: String, pub account: String, pub rgb: u32, pub writable: bool }

/// One occurrence of an event. `start` and `end` are unix seconds; a recurring event's occurrences
/// share `id` and differ in `start`.
#[derive(Clone, Debug, PartialEq)]
pub struct Event { pub id: String, pub calendar: String, pub title: String, pub location: String, pub start: i64, pub end: i64, pub all_day: bool }

/// What a new or edited event says.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft { pub calendar: String, pub title: String, pub location: String, pub start: i64, pub end: i64, pub all_day: bool }

pub use imp::*;

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use block::ConcreteBlock;
    use objc::rc::autoreleasepool;
    use objc::runtime::{BOOL, NO, Object, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[link(name = "EventKit", kind = "framework")]
    unsafe extern "C" {}

    type Id = *mut Object;
    /// EKEntityTypeEvent; EKSpanThisEvent.
    const EVENTS: usize = 0;
    const THIS_EVENT: isize = 0;

    thread_local! {
        /// The one event store, made on first use on the main thread and kept for the app's life.
        static STORE: Cell<Id> = const { Cell::new(std::ptr::null_mut()) };
    }
    /// The permission prompt was answered (its reply comes on another thread).
    static ANSWERED: AtomicBool = AtomicBool::new(false);

    fn store() -> Id {
        STORE.with(|s| {
            if s.get().is_null() {
                let st: Id = unsafe { msg_send![class!(EKEventStore), alloc] };
                let st: Id = unsafe { msg_send![st, init] };
                s.set(st);
            }
            s.get()
        })
    }

    unsafe fn ns(s: &str) -> Id {
        let o: Id = unsafe { msg_send![class!(NSString), alloc] };
        let o: Id = unsafe { msg_send![o, initWithBytes: s.as_ptr() length: s.len() encoding: 4usize] };
        unsafe { msg_send![o, autorelease] }
    }
    unsafe fn rs(o: Id) -> String {
        if o.is_null() { return String::new() }
        let p: *const std::os::raw::c_char = unsafe { msg_send![o, UTF8String] };
        if p.is_null() { String::new() } else { unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned() }
    }
    unsafe fn date(secs: i64) -> Id { unsafe { msg_send![class!(NSDate), dateWithTimeIntervalSince1970: secs as f64] } }
    unsafe fn secs(d: Id) -> i64 { if d.is_null() { 0 } else { let t: f64 = unsafe { msg_send![d, timeIntervalSince1970] }; t.round() as i64 } }
    unsafe fn items(arr: Id) -> Vec<Id> {
        if arr.is_null() { return vec![] }
        let n: usize = unsafe { msg_send![arr, count] };
        (0..n).map(|i| unsafe { msg_send![arr, objectAtIndex: i] }).collect()
    }
    unsafe fn error_text(err: Id) -> String {
        if err.is_null() { return "The calendar did not accept the change.".into() }
        unsafe { rs(msg_send![err, localizedDescription]) }
    }

    pub fn access() -> Access {
        // 0 not determined, 1 restricted, 2 denied, 3 full (authorized), 4 write-only
        let st: isize = unsafe { msg_send![class!(EKEventStore), authorizationStatusForEntityType: EVENTS] };
        match st { 0 => Access::NotAsked, 3 => Access::Full, _ => Access::Denied }
    }

    /// Asks macOS for full calendar access (its own prompt); `answered` turns true once the person
    /// has replied.
    pub fn request() {
        ANSWERED.store(false, Ordering::SeqCst);
        log::info!("calendars: asking for access (status {:?})", access());
        let done = ConcreteBlock::new(|granted: BOOL, err: Id| {
            // macOS turns a request down without asking when the app was not opened as itself (the
            // log says so): the person then never saw a prompt.
            log::info!("calendars: access {}{}", if granted == YES { "granted" } else { "not granted" }, if err.is_null() { String::new() } else { format!(" ({})", unsafe { error_text(err) }) });
            ANSWERED.store(true, Ordering::SeqCst);
        }).copy();
        unsafe {
            let st = store();
            let full: BOOL = msg_send![st, respondsToSelector: sel!(requestFullAccessToEventsWithCompletion:)];
            if full == YES { let _: () = msg_send![st, requestFullAccessToEventsWithCompletion: &*done]; }
            else { let _: () = msg_send![st, requestAccessToEntityType: EVENTS completion: &*done]; }
        }
    }
    pub fn answered() -> bool { ANSWERED.swap(false, Ordering::SeqCst) }
    /// After access is granted the store reads afresh.
    pub fn reset() { unsafe { let _: () = msg_send![store(), reset]; } }

    pub fn calendars() -> Vec<Calendar> {
        if access() != Access::Full { return vec![] }
        autoreleasepool(|| unsafe {
            let srgb: Id = msg_send![class!(NSColorSpace), sRGBColorSpace];
            items(msg_send![store(), calendarsForEntityType: EVENTS]).into_iter().map(|c| {
                let color: Id = msg_send![c, color];
                let color: Id = if color.is_null() { color } else { msg_send![color, colorUsingColorSpace: srgb] };
                let rgb = if color.is_null() { 0x8a8f98 } else {
                    let (r, g, b): (f64, f64, f64) = (msg_send![color, redComponent], msg_send![color, greenComponent], msg_send![color, blueComponent]);
                    let q = |v: f64| (v.clamp(0., 1.) * 255.).round() as u32;
                    q(r) << 16 | q(g) << 8 | q(b)
                };
                let source: Id = msg_send![c, source];
                let writable: BOOL = msg_send![c, allowsContentModifications];
                Calendar { id: rs(msg_send![c, calendarIdentifier]), title: rs(msg_send![c, title]), account: rs(msg_send![source, title]), rgb, writable: writable == YES }
            }).collect()
        })
    }

    unsafe fn read(e: Id) -> Event {
        unsafe {
            let cal: Id = msg_send![e, calendar];
            let all_day: BOOL = msg_send![e, isAllDay];
            Event {
                id: rs(msg_send![e, eventIdentifier]), calendar: if cal.is_null() { String::new() } else { rs(msg_send![cal, calendarIdentifier]) },
                title: rs(msg_send![e, title]), location: rs(msg_send![e, location]),
                start: secs(msg_send![e, startDate]), end: secs(msg_send![e, endDate]), all_day: all_day == YES,
            }
        }
    }

    unsafe fn matching(from: i64, to: i64) -> Vec<Id> {
        unsafe {
            let p: Id = msg_send![store(), predicateForEventsWithStartDate: date(from) endDate: date(to) calendars: std::ptr::null_mut::<Object>()];
            items(msg_send![store(), eventsMatchingPredicate: p])
        }
    }

    /// Every occurrence between `from` and `to` (unix seconds), in every calendar.
    pub fn events(from: i64, to: i64) -> Vec<Event> {
        if access() != Access::Full { return vec![] }
        autoreleasepool(|| unsafe { matching(from, to).into_iter().map(|e| read(e)).collect() })
    }

    /// The occurrence of `id` that starts at `start` (a recurring event's own one).
    unsafe fn occurrence(id: &str, start: i64) -> Option<Id> {
        unsafe { matching(start - 1, start + 86_400).into_iter().find(|&e| rs(msg_send![e, eventIdentifier]) == id && secs(msg_send![e, startDate]) == start) }
    }

    /// One occurrence, looked up afresh.
    pub fn event(id: &str, start: i64) -> Option<Event> {
        if access() != Access::Full { return None }
        autoreleasepool(|| unsafe { occurrence(id, start).map(|e| read(e)) })
    }

    /// Writes `d` as a new event (`old` None) or over the occurrence `old` (id, start): that
    /// occurrence only, never the rest of a series.
    pub fn save(old: Option<(&str, i64)>, d: &Draft) -> Result<(), String> {
        autoreleasepool(|| unsafe {
            let st = store();
            let e: Id = match old {
                Some((id, start)) => occurrence(id, start).ok_or("That event is no longer in the calendar.")?,
                None => msg_send![class!(EKEvent), eventWithEventStore: st],
            };
            let cal: Id = msg_send![st, calendarWithIdentifier: ns(&d.calendar)];
            if cal.is_null() { return Err("That calendar is no longer on this Mac.".into()) }
            let _: () = msg_send![e, setCalendar: cal];
            let _: () = msg_send![e, setTitle: ns(&d.title)];
            let _: () = msg_send![e, setLocation: if d.location.is_empty() { std::ptr::null_mut() } else { ns(&d.location) }];
            let _: () = msg_send![e, setAllDay: if d.all_day { YES } else { NO }];
            let _: () = msg_send![e, setStartDate: date(d.start)];
            let _: () = msg_send![e, setEndDate: date(d.end)];
            let mut err: Id = std::ptr::null_mut();
            let ok: BOOL = msg_send![st, saveEvent: e span: THIS_EVENT commit: YES error: &mut err];
            if ok == YES { Ok(()) } else { Err(error_text(err)) }
        })
    }

    /// Deletes the occurrence `id` at `start` (only that one of a series).
    pub fn delete(id: &str, start: i64) -> Result<(), String> {
        autoreleasepool(|| unsafe {
            let e = occurrence(id, start).ok_or("That event is no longer in the calendar.")?;
            let mut err: Id = std::ptr::null_mut();
            let ok: BOOL = msg_send![store(), removeEvent: e span: THIS_EVENT commit: YES error: &mut err];
            if ok == YES { Ok(()) } else { Err(error_text(err)) }
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;
    pub fn access() -> Access { Access::Unsupported }
    pub fn request() {}
    pub fn answered() -> bool { false }
    pub fn reset() {}
    pub fn calendars() -> Vec<Calendar> { vec![] }
    pub fn events(_: i64, _: i64) -> Vec<Event> { vec![] }
    pub fn event(_: &str, _: i64) -> Option<Event> { None }
    pub fn save(_: Option<(&str, i64)>, _: &Draft) -> Result<(), String> { Err("Calendar sync needs macOS.".into()) }
    pub fn delete(_: &str, _: i64) -> Result<(), String> { Err("Calendar sync needs macOS.".into()) }
}
