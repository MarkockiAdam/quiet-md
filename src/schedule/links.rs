//! 课表 ⇄ 笔记 (fork feature): a note can be linked to what it was taken for, one lecture of a
//! class or one assignment, exam or event (`Meta::event`, a `Link` key). The schedule's details
//! card lists a lecture's notes and makes new ones; a linked note's header carries a chip back to
//! its day, and its calendar button links or unlinks it. The link lives on the note, so it moves
//! with it and goes when it is deleted.

use super::model::Link;
use super::ScheduleEvent;
use crate::app::{View, Workspace};
use crate::ui::{TipExt, TitleBarExt};
use gpui::*;
use std::collections::HashMap;

impl Workspace {
    pub(crate) fn on_schedule_event(&mut self, ev: &ScheduleEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            ScheduleEvent::Open(id) => self.open(id, window, cx),
            ScheduleEvent::NewNote { link, title, course } => self.new_note_for(link.clone(), title.clone(), course.clone(), window, cx),
            ScheduleEvent::Link { note, link } => self.link_note(note, link.as_ref(), cx),
        }
    }

    /// 新建笔记 from the schedule: a document titled for the lecture, linked to it, in the folder named
    /// after its course when there is one (else the default place), and opened.
    fn new_note_for(&mut self, link: Link, title: String, course: String, window: &mut Window, cx: &mut Context<Self>) {
        let (l, t, c) = (link.clone(), title.clone(), course.clone());
        if !self.save_before(cx, move |w, window, cx| w.new_note_for(l.clone(), t.clone(), c.clone(), window, cx)) { return }
        let parent = (!course.is_empty()).then(|| self.entries.iter()
            .find(|e| e.meta.folder.is_some() && e.note.is_none() && e.title.trim().eq_ignore_ascii_case(course.trim())).map(|e| e.id.clone())).flatten();
        let doc = crate::model::Document { title: crate::model::RichText::plain(title), ..Default::default() };
        match self.store.create_from_in(&doc, parent.as_deref()) {
            Ok(id) => {
                if let Err(e) = self.store.update_meta(&id, |m| m.event = Some(link.key())) { log::warn!("schedule: linking the new note failed: {e}"); }
                self.entries.set(self.store.list());
                self.sort();
                if let Some(p) = &parent { self.expanded.insert(p.clone()); }
                self.open(&id, window, cx);
                if let Some(ed) = &self.editor { let f = ed.read(cx).focus.clone(); window.focus(&f, cx); }
            }
            Err(e) => { log::warn!("schedule: new note failed: {e}"); self.show_toast(crate::settings::tr("新建失败", "Could not create the note"), cx) }
        }
        cx.notify();
    }

    pub(crate) fn link_note(&mut self, note: &str, link: Option<&Link>, cx: &mut Context<Self>) {
        let key = link.map(Link::key);
        if self.store.update_meta(note, |m| m.event = key.clone()).is_ok()
            && let Some(e) = self.entries.edit().iter_mut().find(|e| e.id == note)
        {
            e.meta.event = key.clone();
        }
        self.push_schedule_links(cx);
        self.show_toast(if key.is_some() { crate::settings::tr("已关联到课表", "Linked to the schedule") } else { crate::settings::tr("已取消关联", "Link removed") }, cx);
        cx.notify();
    }

    /// Hands the schedule what it shows of the notes: which are linked to what, and every note for
    /// 关联笔记 (most recently opened first).
    pub(crate) fn push_schedule_links(&mut self, cx: &mut Context<Self>) {
        let mut links: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut notes: Vec<(u64, String, String)> = vec![];
        let mut folders: Vec<(String, Option<String>)> = vec![];
        for e in self.entries.iter() {
            // A folder is a course: its name and colour are what the schedule suggests (not the
            // top-level folders, which are the libraries themselves).
            if e.meta.folder.is_some() && e.note.is_none() {
                let name = e.title.trim().to_string();
                if crate::shared_store::top_of_row(&e.id).is_none() && !name.is_empty() && !folders.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                    folders.push((name, e.meta.color.clone()));
                }
                continue
            }
            if let Some(k) = &e.meta.event { links.entry(k.clone()).or_default().push((e.id.clone(), e.title.clone())); }
            notes.push((e.meta.opened.max(e.meta.modified), e.id.clone(), e.title.clone()));
        }
        notes.sort_by(|a, b| b.0.cmp(&a.0));
        let notes = notes.into_iter().map(|(_, id, t)| (id, t)).collect();
        folders.sort_by_key(|(n, _)| n.to_lowercase());
        self.schedule.update(cx, |s, _| { s.links = links; s.notes = notes; s.folders = folders; });
    }

    /// The open note's link to the schedule, as a chip for its header: a click shows that day.
    pub(crate) fn calendar_chip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.current.as_ref()?;
        let key = self.entries.iter().find(|e| &e.id == id)?.meta.event.clone()?;
        let link = Link::parse(&key)?;
        let (d, ink) = self.schedule.read(cx).describe(&link)?;
        let day = d.day;
        Some(div().id("cal-chip").no_drag().flex_none().h(px(24.)).px(px(9.)).rounded_full().bg(ink.opacity(if crate::theme::dark() { 0.24 } else { 0.12 }))
            .flex().items_center().gap(px(6.)).cursor_pointer().hover(|s| s.opacity(0.85))
            .child(crate::ui::icon("calendar", 13., ink))
            .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(ink).whitespace_nowrap().child(format!("{} · {}", d.title, d.sub)))
            // × unlinks the note right here (the note stays).
            .child(div().id("cal-chip-unlink").size(px(16.)).ml(px(1.)).mr(px(-4.)).rounded_full().flex().items_center().justify_center()
                .hover(|s| s.bg(ink.opacity(0.22))).child(crate::ui::icon("x", 10., ink))
                .tip(crate::settings::tr("取消关联", "Unlink from the schedule"), None)
                .on_click(cx.listener(|w, _, _, cx| { cx.stop_propagation(); if let Some(id) = w.current.clone() { w.link_note(&id, None, cx); } })))
            .tip(crate::settings::tr("在课表中查看", "Show in the schedule"), None)
            .on_click(cx.listener(move |w, _, _, cx| {
                w.show_view(View::Schedule, cx);
                w.schedule.update(cx, |s, cx| s.goto(day, cx));
            }))
            .into_any_element())
    }

    /// The header's calendar button: 关联到课表 for the open note.
    pub(crate) fn calendar_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::header_icon("doc-calendar", "calendar")
            .tip(crate::settings::tr("课表与作业", "Tasks and schedule"), None)
            .on_click(cx.listener(|w, _, window, cx| w.open_note_schedule(window, cx)))
    }

    /// The open note's schedule sheet: its tasks, what it is linked to. A task added there takes the
    /// note's folder as its course (when the note sits in one below the library's top).
    fn open_note_schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.current.clone() else { return };
        let course = self.entries.iter().find(|e| e.id == id).and_then(|e| e.meta.parent.clone())
            .filter(|p| crate::shared_store::top_of_row(p).is_none())
            .and_then(|p| self.entries.iter().find(|e| e.id == p).map(|e| e.title.trim().to_string()))
            .unwrap_or_default();
        self.push_schedule_links(cx);
        self.schedule.update(cx, |s, cx| s.pick_for(id, course, window, cx));
    }

    /// The open note's tasks still to do, as a pill for its header: a click opens its schedule sheet.
    pub(crate) fn tasks_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.current.as_ref()?;
        let n = self.schedule.read(cx).data.items.iter().filter(|i| !i.done && i.note.as_deref() == Some(id.as_str())).count();
        if n == 0 { return None }
        let ink = crate::design::color::foreground();
        Some(div().id("tasks-pill").no_drag().flex_none().h(px(24.)).px(px(9.)).rounded_full().bg(crate::design::color::accent())
            .flex().items_center().gap(px(6.)).cursor_pointer().hover(|s| s.opacity(0.85))
            .child(crate::ui::icon("list-todo", 13., ink))
            .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(ink).whitespace_nowrap().child(format!("{n} to do")))
            .tip(crate::settings::tr("这篇笔记的作业", "Tasks from this note"), None)
            .on_click(cx.listener(|w, _, window, cx| w.open_note_schedule(window, cx)))
            .into_any_element())
    }
}
