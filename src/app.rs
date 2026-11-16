//! Application state and event handling.
use crate::backend::{auth, auth::DeviceCode, Backend};
use crate::input::Input;
use crate::model::*;
use chrono::{DateTime, Datelike, Duration as CDuration, Local, NaiveDate, NaiveTime, Timelike, Utc};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

pub enum Ev {
    Input(Event),
    Tick,
    DeviceCode(Result<DeviceCode, String>),
    LoginDone(Result<(), String>),
    Me(User),
    Chats(Vec<Chat>),
    Messages { conv: ConvRef, page: Page, older: bool },
    Teams(Vec<Team>),
    Events(NaiveDate, Vec<CalEvent>),
    Presence(HashMap<String, Availability>),
    People(String, Vec<User>),
    SearchHits(String, Vec<User>, Vec<SearchHit>),
    ChatCreated(Chat),
    Files(Vec<DriveItem>),
    ImageLoaded { url: String, img: Result<image::DynamicImage, String> },
    Done { toast: Option<String>, refresh: Option<ConvRef>, calendar: bool },
    Error(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Activity,
    Chat,
    Teams,
    Calendar,
    Calls,
    Files,
}

pub const VIEWS: [View; 6] = [View::Activity, View::Chat, View::Teams, View::Calendar, View::Calls, View::Files];

impl View {
    pub fn label(self) -> &'static str {
        match self {
            View::Activity => "Activity",
            View::Chat => "Chat",
            View::Teams => "Teams",
            View::Calendar => "Calendar",
            View::Calls => "Calls",
            View::Files => "OneDrive",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            View::Activity => "🔔",
            View::Chat => "💬",
            View::Teams => "👥",
            View::Calendar => "📅",
            View::Calls => "📞",
            View::Files => "📁",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    List,
    Messages,
    Compose,
}

/// Clickable regions recorded while rendering.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Rail(View),
    Help,
    Search,
    Profile,
    List(usize),
    ListArea,
    Message(usize),
    Messages,
    Compose,
    Call(bool),
    NewItem,
    CalNav(i32),
    CalEvent(usize),
    FileRow(usize),
    FileUp,
    Backdrop,
    ModalBody,
    ModalItem(usize),
}

pub struct Flat<'a> {
    pub m: &'a Message,
    pub parent: Option<&'a str>,
}

pub struct Conv {
    pub target: ConvRef,
    pub title: String,
    pub subtitle: String,
    pub msgs: Vec<Message>,
    /// Index into `flat()`.
    pub sel: Option<usize>,
    /// Lines scrolled up from the bottom.
    pub scroll: usize,
    /// Ask the renderer to bring the selection into view.
    pub follow: bool,
    pub loaded: bool,
    pub loading: bool,
    pub cursor: Option<String>,
    pub loading_older: bool,
    pub want_older: bool,
}

impl Conv {
    fn new(target: ConvRef, title: String, subtitle: String) -> Self {
        Self { target, title, subtitle, msgs: vec![], sel: None, scroll: 0, follow: false, loaded: false, loading: false, cursor: None, loading_older: false, want_older: false }
    }

    pub fn is_channel(&self) -> bool {
        matches!(self.target, ConvRef::Channel { .. })
    }

    /// Posts and their replies in display order.
    pub fn flat(&self) -> Vec<Flat<'_>> {
        let mut v = vec![];
        for m in &self.msgs {
            v.push(Flat { m, parent: None });
            for r in &m.replies {
                v.push(Flat { m: r, parent: Some(&m.id) });
            }
        }
        v
    }

    fn apply(&mut self, mut page: Page, older: bool) {
        let sel_id = self.sel.and_then(|i| self.flat().get(i).map(|f| f.m.id.clone()));
        if older {
            self.loading_older = false;
            self.cursor = page.next.take();
            page.msgs.retain(|n| !self.msgs.iter().any(|m| m.id == n.id));
        } else {
            self.loading = false;
            if !self.loaded {
                self.cursor = page.next.take();
            }
            self.loaded = true;
            let fresh: HashSet<&str> = page.msgs.iter().map(|m| m.id.as_str()).collect();
            self.msgs.retain(|m| !m.id.starts_with("pending-") && !fresh.contains(m.id.as_str()));
            for m in &mut self.msgs {
                m.replies.retain(|r| !r.id.starts_with("pending-"));
            }
        }
        self.msgs.extend(page.msgs);
        self.msgs.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.id.cmp(&b.id)));
        for m in &mut self.msgs {
            m.replies.sort_by_key(|r| r.created);
        }
        if let Some(id) = sel_id {
            self.sel = self.flat().iter().position(|f| f.m.id == id);
        }
    }
}

#[derive(Clone)]
pub struct ReplyCtx {
    /// Root post id (channels) or quoted message id (chats).
    pub id: String,
    pub who: String,
    pub text: String,
}

#[derive(Clone)]
pub struct ActivityItem {
    pub title: String,
    pub preview: String,
    pub when: Option<DateTime<Utc>>,
    pub unread: bool,
    pub mention: bool,
    pub conv: ConvRef,
    pub key: String,
}

#[derive(Clone, Copy)]
pub enum TeamRow {
    Team(usize),
    Channel(usize, usize),
}

#[derive(Clone)]
pub enum SearchItem {
    Chat { id: String, title: String },
    Channel { team: usize, channel: usize, title: String },
    Person(User),
    Message(SearchHit),
}

pub const REACTIONS: [&str; 6] = ["👍", "❤️", "😆", "😮", "😢", "😠"];
pub const PRESENCE_EXTRA: [&str; 3] = ["Reset status", "Set status message…", "Sign out"];
pub const MEETING_FIELDS: [&str; 5] = ["Title", "Date (YYYY-MM-DD)", "Start (HH:MM)", "Duration (minutes)", "Invite (emails, comma separated)"];

pub enum Modal {
    None,
    Help,
    Search { input: Input, results: Vec<SearchItem>, sel: usize, loading: bool },
    NewChat { input: Input, results: Vec<User>, sel: usize, picked: Vec<User> },
    React { sel: usize },
    Presence { sel: usize },
    StatusMsg { input: Input },
    ConfirmDelete,
    Event(usize),
    NewMeeting { fields: Vec<Input>, focus: usize },
    Image { title: String, url: String, gif: bool, img: Option<image::DynamicImage>, error: Option<String> },
}

pub struct App {
    pub backend: Arc<dyn Backend>,
    tx: UnboundedSender<Ev>,
    pub demo: bool,
    pub bell: bool,
    pub quit: bool,
    pub exit_msg: Option<String>,

    pub me: User,
    pub my_presence: Availability,
    pub status_msg: String,
    pub presence: HashMap<String, Availability>,

    pub view: View,
    pub focus: Focus,
    pub modal: Modal,
    pub toast: Option<(String, Instant, bool)>,
    pub hits: Vec<(Rect, Hit)>,
    pub spinner: usize,

    pub chats: Vec<Chat>,
    pub chats_loaded: bool,
    chats_inflight: bool,
    pub chat_sel: usize,
    pub chat_conv: Option<Conv>,

    pub teams: Vec<Team>,
    pub teams_loaded: bool,
    pub collapsed: HashSet<String>,
    pub team_sel: usize,
    pub chan_conv: Option<Conv>,

    pub compose: Input,
    pub editing: Option<MsgId>,
    pub reply: Option<ReplyCtx>,
    drafts: HashMap<ConvRef, String>,
    pending_seq: u64,
    img_seq: u64,

    pub activity: Vec<ActivityItem>,
    mentions: Vec<ActivityItem>,
    pub act_sel: usize,

    pub week_start: NaiveDate,
    pub events: Vec<CalEvent>,
    pub events_loaded: bool,
    pub ev_sel: usize,

    pub call_sel: usize,

    pub files: Vec<DriveItem>,
    pub files_loaded: bool,
    pub file_sel: usize,
    pub file_path: Vec<(Option<String>, String)>,

    search_due: Option<Instant>,
    conv_inflight: bool,
    last_chats: Instant,
    last_conv: Instant,
    last_presence: Instant,
    last_events: Instant,
}

fn monday(d: NaiveDate) -> NaiveDate {
    d - CDuration::days(d.weekday().num_days_from_monday() as i64)
}

pub fn first_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|w| w.contains("https://") || w.contains("http://"))
        .map(|w| w[w.find("http").unwrap()..].trim_end_matches([')', ',', '.', ';', '>', ']']).to_string())
}

impl App {
    pub fn new(backend: Arc<dyn Backend>, tx: UnboundedSender<Ev>, demo: bool) -> Self {
        let now = Instant::now();
        Self {
            backend,
            tx,
            demo,
            bell: true,
            quit: false,
            exit_msg: None,
            me: User::default(),
            my_presence: Availability::Unknown,
            status_msg: String::new(),
            presence: HashMap::new(),
            view: View::Chat,
            focus: Focus::List,
            modal: Modal::None,
            toast: None,
            hits: vec![],
            spinner: 0,
            chats: vec![],
            chats_loaded: false,
            chats_inflight: false,
            chat_sel: 0,
            chat_conv: None,
            teams: vec![],
            teams_loaded: false,
            collapsed: HashSet::new(),
            team_sel: 0,
            chan_conv: None,
            compose: Input::default(),
            editing: None,
            reply: None,
            drafts: HashMap::new(),
            pending_seq: 0,
            img_seq: 0,
            activity: vec![],
            mentions: vec![],
            act_sel: 0,
            week_start: monday(Local::now().date_naive()),
            events: vec![],
            events_loaded: false,
            ev_sel: 0,
            call_sel: 0,
            files: vec![],
            files_loaded: false,
            file_sel: 0,
            file_path: vec![(None, "My files".into())],
            search_due: None,
            conv_inflight: false,
            last_chats: now,
            last_conv: now,
            last_presence: now,
            last_events: now,
        }
    }

    pub fn start(&mut self) {
        self.spawn(|b| async move {
            match b.me().await {
                Ok(u) => Ev::Me(u),
                Err(e) => Ev::Error(format!("Couldn't load your profile: {e}")),
            }
        });
        self.load_chats();
        self.load_teams();
        self.load_events();
    }

    // ---- background work -------------------------------------------------

    fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce(Arc<dyn Backend>) -> Fut,
        Fut: Future<Output = Ev> + Send + 'static,
    {
        let fut = f(self.backend.clone());
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(fut.await);
        });
    }

    /// Run an action and report `ok` as a toast, refreshing what it touched.
    fn act<F, Fut>(&self, what: &'static str, ok: Option<String>, refresh: Option<ConvRef>, calendar: bool, f: F)
    where
        F: FnOnce(Arc<dyn Backend>) -> Fut,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let fut = f(self.backend.clone());
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(match fut.await {
                Ok(()) => Ev::Done { toast: ok, refresh, calendar },
                Err(e) => Ev::Error(format!("{what}: {e}")),
            });
        });
    }

    fn load_chats(&mut self) {
        if self.chats_inflight {
            return;
        }
        self.chats_inflight = true;
        self.last_chats = Instant::now();
        self.spawn(|b| async move {
            match b.chats().await {
                Ok(c) => Ev::Chats(c),
                Err(e) => Ev::Error(format!("Couldn't load chats: {e}")),
            }
        });
    }

    fn load_teams(&mut self) {
        self.spawn(|b| async move {
            match b.teams().await {
                Ok(t) => Ev::Teams(t),
                Err(e) => Ev::Error(format!("Couldn't load teams: {e}")),
            }
        });
    }

    fn load_events(&mut self) {
        self.last_events = Instant::now();
        let week = self.week_start;
        let start = week.and_time(NaiveTime::MIN).and_local_timezone(Local).earliest().map(|d| d.with_timezone(&Utc)).unwrap_or_else(Utc::now);
        self.spawn(move |b| async move {
            match b.events(start, start + CDuration::days(7)).await {
                Ok(mut e) => {
                    e.sort_by_key(|e| e.start);
                    Ev::Events(week, e)
                }
                Err(e) => Ev::Error(format!("Couldn't load the calendar: {e}")),
            }
        });
    }

    fn load_files(&mut self) {
        self.files_loaded = false;
        let folder = self.file_path.last().and_then(|p| p.0.clone());
        self.spawn(|b| async move {
            match b.files(folder).await {
                Ok(f) => Ev::Files(f),
                Err(e) => Ev::Error(format!("Couldn't load files: {e}")),
            }
        });
    }

    fn load_presence(&mut self) {
        self.last_presence = Instant::now();
        if self.me.id.is_empty() {
            return;
        }
        let mut ids: Vec<String> = vec![self.me.id.clone()];
        for c in &self.chats {
            for m in &c.members {
                if !m.id.is_empty() && !ids.contains(&m.id) && ids.len() < 200 {
                    ids.push(m.id.clone());
                }
            }
        }
        self.spawn(|b| async move {
            match b.presence(ids).await {
                Ok(p) => Ev::Presence(p),
                // Presence is optional (needs Presence.Read.All); stay quiet if unavailable.
                Err(_) => Ev::Presence(HashMap::new()),
            }
        });
    }

    fn load_messages(&mut self, target: ConvRef, cursor: Option<String>) {
        self.last_conv = Instant::now();
        let older = cursor.is_some();
        if !older {
            self.conv_inflight = true;
        }
        self.spawn(move |b| async move {
            match b.messages(&target, cursor).await {
                Ok(page) => Ev::Messages { conv: target, page, older },
                Err(e) => Ev::Error(format!("Couldn't load messages: {e}")),
            }
        });
    }

    // ---- helpers ---------------------------------------------------------

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now(), false));
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now(), true));
    }

    pub fn conv(&self) -> Option<&Conv> {
        match self.view {
            View::Chat => self.chat_conv.as_ref(),
            View::Teams => self.chan_conv.as_ref(),
            _ => None,
        }
    }

    fn conv_mut(&mut self) -> Option<&mut Conv> {
        match self.view {
            View::Chat => self.chat_conv.as_mut(),
            View::Teams => self.chan_conv.as_mut(),
            _ => None,
        }
    }

    pub fn team_rows(&self) -> Vec<TeamRow> {
        let mut rows = vec![];
        for (ti, t) in self.teams.iter().enumerate() {
            rows.push(TeamRow::Team(ti));
            if !self.collapsed.contains(&t.id) {
                rows.extend((0..t.channels.len()).map(|ci| TeamRow::Channel(ti, ci)));
            }
        }
        rows
    }

    /// People you have 1:1 chats with — the Calls speed dial and people suggestions.
    pub fn contacts(&self) -> Vec<User> {
        let mut out: Vec<User> = vec![];
        for c in self.chats.iter().filter(|c| c.kind == ChatKind::OneOnOne) {
            for u in c.others(&self.me.id) {
                if !u.id.is_empty() && !out.iter().any(|o| o.id == u.id) {
                    out.push(u.clone());
                }
            }
        }
        out
    }

    pub fn unread_chats(&self) -> usize {
        self.chats.iter().filter(|c| c.unread).count()
    }

    pub fn list_len(&self) -> usize {
        match self.view {
            View::Activity => self.activity.len(),
            View::Chat => self.chats.len(),
            View::Teams => self.team_rows().len(),
            View::Calendar => self.events.len(),
            View::Calls => self.contacts().len(),
            View::Files => self.files.len(),
        }
    }

    fn sel_mut(&mut self) -> &mut usize {
        match self.view {
            View::Activity => &mut self.act_sel,
            View::Chat => &mut self.chat_sel,
            View::Teams => &mut self.team_sel,
            View::Calendar => &mut self.ev_sel,
            View::Calls => &mut self.call_sel,
            View::Files => &mut self.file_sel,
        }
    }

    fn move_sel(&mut self, delta: i64) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let sel = self.sel_mut();
        *sel = (*sel as i64 + delta).clamp(0, len as i64 - 1) as usize;
    }

    fn open_url(&mut self, url: &str) {
        if url.is_empty() {
            return;
        }
        if self.demo && url.contains("contoso.example") {
            self.toast(format!("Demo mode — would open {url}"));
            return;
        }
        match open::that_detached(url) {
            Ok(()) => self.toast("Opened in your browser"),
            Err(_) => self.error(format!("Couldn't open a browser. Link: {url}")),
        }
    }

    fn rebuild_activity(&mut self) {
        let me = self.me.id.clone();
        let mut items: Vec<ActivityItem> = self
            .chats
            .iter()
            .filter(|c| c.last_time.is_some() && !c.preview_from.id.is_empty() && c.preview_from.id != me)
            .map(|c| ActivityItem {
                title: match c.kind {
                    ChatKind::OneOnOne => format!("{} sent a message", c.preview_from.name),
                    _ => format!("{} in {}", c.preview_from.name, c.title(&me)),
                },
                preview: c.preview.clone(),
                when: c.last_time,
                unread: c.unread,
                mention: false,
                conv: ConvRef::Chat(c.id.clone()),
                key: format!("chat-{}", c.id),
            })
            .collect();
        items.extend(self.mentions.iter().cloned());
        items.sort_by(|a, b| b.when.cmp(&a.when));
        self.activity = items;
        self.act_sel = self.act_sel.min(self.activity.len().saturating_sub(1));
    }

    fn collect_mentions(&mut self, conv: &ConvRef, title: &str) {
        let me = self.me.id.clone();
        if me.is_empty() {
            return;
        }
        let Some(c) = [&self.chat_conv, &self.chan_conv].into_iter().flatten().find(|c| &c.target == conv) else { return };
        let mut found = vec![];
        for f in c.flat() {
            if f.m.mentions.contains(&me) && f.m.from.id != me && !f.m.deleted {
                found.push(ActivityItem {
                    title: format!("{} mentioned you in {title}", f.m.from.name),
                    preview: f.m.body.lines().next().unwrap_or("").to_string(),
                    when: Some(f.m.created),
                    unread: false,
                    mention: true,
                    conv: conv.clone(),
                    key: format!("mention-{}", f.m.id),
                });
            }
        }
        let mut changed = false;
        for item in found {
            if !self.mentions.iter().any(|m| m.key == item.key) {
                self.mentions.push(item);
                changed = true;
            }
        }
        if changed {
            self.rebuild_activity();
        }
    }

    // ---- navigation ------------------------------------------------------

    fn stash_draft(&mut self) {
        let text = self.compose.take();
        self.editing = None;
        self.reply = None;
        if let Some(target) = self.conv().map(|c| c.target.clone()) {
            if text.trim().is_empty() {
                self.drafts.remove(&target);
            } else {
                self.drafts.insert(target, text);
            }
        }
    }

    fn restore_draft(&mut self) {
        let draft = self.conv().and_then(|c| self.drafts.get(&c.target).cloned()).unwrap_or_default();
        self.compose.set(&draft);
    }

    pub fn set_view(&mut self, v: View) {
        if v == self.view {
            return;
        }
        self.stash_draft();
        self.view = v;
        self.restore_draft();
        self.focus = if self.conv().is_some() { Focus::Compose } else { Focus::List };
        match v {
            View::Files if !self.files_loaded && self.files.is_empty() => self.load_files(),
            View::Chat => self.mark_open_chat_read(),
            _ => {}
        }
        if let Some(t) = self.conv().map(|c| c.target.clone()) {
            self.load_messages(t, None);
        }
    }

    fn mark_open_chat_read(&mut self) {
        let Some(ConvRef::Chat(id)) = self.chat_conv.as_ref().map(|c| c.target.clone()) else { return };
        if let Some(c) = self.chats.iter_mut().find(|c| c.id == id && c.unread) {
            c.unread = false;
            self.spawn(|b| async move {
                let _ = b.mark_read(&id).await;
                Ev::Done { toast: None, refresh: None, calendar: false }
            });
            self.rebuild_activity();
        }
    }

    fn open_conv(&mut self, target: ConvRef, title: String, subtitle: String) {
        self.stash_draft();
        let conv = Conv::new(target.clone(), title, subtitle);
        match target {
            ConvRef::Chat(_) => {
                self.view = View::Chat;
                self.chat_conv = Some(conv);
            }
            ConvRef::Channel { .. } => {
                self.view = View::Teams;
                self.chan_conv = Some(conv);
            }
        }
        if let Some(c) = self.conv_mut() {
            c.loading = true;
        }
        self.restore_draft();
        self.focus = Focus::Compose;
        self.load_messages(target, None);
        self.mark_open_chat_read();
    }

    pub fn open_chat_id(&mut self, id: &str) {
        let (title, subtitle) = match self.chats.iter().position(|c| c.id == id) {
            Some(i) => {
                self.chat_sel = i;
                let c = &self.chats[i];
                let sub = match c.kind {
                    ChatKind::OneOnOne => c.others(&self.me.id).next().map(|u| u.email.clone()).unwrap_or_default(),
                    _ => format!("{} people", c.members.len()),
                };
                (c.title(&self.me.id), sub)
            }
            None => ("Chat".to_string(), String::new()),
        };
        self.open_conv(ConvRef::Chat(id.to_string()), title, subtitle);
    }

    fn open_channel(&mut self, ti: usize, ci: usize) {
        let Some(team) = self.teams.get(ti) else { return };
        let Some(ch) = team.channels.get(ci) else { return };
        let target = ConvRef::Channel { team: team.id.clone(), channel: ch.id.clone() };
        let (title, sub) = (format!("{} › {}", team.name, ch.name), ch.description.clone());
        if let Some(row) = self.team_rows().iter().position(|r| matches!(r, TeamRow::Channel(a, b) if *a == ti && *b == ci)) {
            self.team_sel = row;
        }
        self.open_conv(target, title, sub);
    }

    fn open_target(&mut self, conv: &ConvRef) {
        match conv {
            ConvRef::Chat(id) => self.open_chat_id(id),
            ConvRef::Channel { team, channel } => {
                let found = self.teams.iter().enumerate().find_map(|(ti, t)| (t.id == *team).then(|| t.channels.iter().position(|c| c.id == *channel).map(|ci| (ti, ci))).flatten());
                match found {
                    Some((ti, ci)) => {
                        self.collapsed.remove(&self.teams[ti].id.clone());
                        self.open_channel(ti, ci);
                    }
                    None => self.open_conv(conv.clone(), "Channel".into(), String::new()),
                }
            }
        }
    }

    fn open_person(&mut self, u: User) {
        let existing = self.chats.iter().find(|c| c.kind == ChatKind::OneOnOne && c.members.iter().any(|m| m.id == u.id)).map(|c| c.id.clone());
        match existing {
            Some(id) => self.open_chat_id(&id),
            None => self.create_chat(vec![u]),
        }
    }

    fn create_chat(&mut self, users: Vec<User>) {
        self.toast("Starting chat…");
        self.spawn(|b| async move {
            match b.create_chat(users).await {
                Ok(c) => Ev::ChatCreated(c),
                Err(e) => Ev::Error(format!("Couldn't start the chat: {e}")),
            }
        });
    }

    fn activate_list(&mut self) {
        match self.view {
            View::Activity => {
                if let Some(item) = self.activity.get(self.act_sel).cloned() {
                    self.open_target(&item.conv);
                }
            }
            View::Chat => {
                if let Some(id) = self.chats.get(self.chat_sel).map(|c| c.id.clone()) {
                    self.open_chat_id(&id);
                }
            }
            View::Teams => match self.team_rows().get(self.team_sel).copied() {
                Some(TeamRow::Team(ti)) => {
                    let id = self.teams[ti].id.clone();
                    if !self.collapsed.remove(&id) {
                        self.collapsed.insert(id);
                    }
                }
                Some(TeamRow::Channel(ti, ci)) => self.open_channel(ti, ci),
                None => {}
            },
            View::Calendar => {
                if self.ev_sel < self.events.len() {
                    self.modal = Modal::Event(self.ev_sel);
                }
            }
            View::Calls => self.start_call(false),
            View::Files => {
                if let Some(item) = self.files.get(self.file_sel).cloned() {
                    if item.folder {
                        self.file_path.push((Some(item.id), item.name));
                        self.file_sel = 0;
                        self.files.clear();
                        self.load_files();
                    } else {
                        self.open_url(&item.web_url);
                    }
                }
            }
        }
    }

    fn files_up(&mut self) {
        if self.file_path.len() > 1 {
            self.file_path.pop();
            self.file_sel = 0;
            self.files.clear();
            self.load_files();
        }
    }

    fn shift_week(&mut self, delta: i32) {
        self.week_start = if delta == 0 { monday(Local::now().date_naive()) } else { self.week_start + CDuration::days(7 * delta as i64) };
        self.events.clear();
        self.events_loaded = false;
        self.ev_sel = 0;
        self.load_events();
    }

    /// Audio/video cannot run in a terminal: hand the call to Teams via a deep link.
    fn start_call(&mut self, video: bool) {
        let emails: Vec<String> = match self.view {
            View::Calls => self.contacts().get(self.call_sel).map(|u| vec![u.email.clone()]).unwrap_or_default(),
            _ => match self.chat_conv.as_ref().map(|c| &c.target) {
                Some(ConvRef::Chat(id)) if self.view == View::Chat => {
                    self.chats.iter().find(|c| &c.id == id).map(|c| c.others(&self.me.id).map(|u| u.email.clone()).collect()).unwrap_or_default()
                }
                _ => vec![],
            },
        };
        let emails: Vec<String> = emails.into_iter().filter(|e| !e.is_empty()).collect();
        if emails.is_empty() {
            self.error("No one to call here — pick a chat or a contact");
            return;
        }
        let mut url = format!("https://teams.microsoft.com/l/call/0/0?users={}", emails.join(","));
        if video {
            url.push_str("&withVideo=true");
        }
        if self.demo {
            self.toast(format!("Demo mode — would call {}", emails.join(", ")));
        } else {
            self.open_url(&url);
        }
    }

    // ---- messaging -------------------------------------------------------

    fn mention_candidates(&self) -> Vec<User> {
        let Some(conv) = self.conv() else { return vec![] };
        let mut out: Vec<User> = vec![];
        match &conv.target {
            ConvRef::Chat(id) => {
                if let Some(c) = self.chats.iter().find(|c| &c.id == id) {
                    out.extend(c.others(&self.me.id).cloned());
                }
            }
            ConvRef::Channel { .. } => {
                for f in conv.flat() {
                    if f.m.from.id != self.me.id && !f.m.from.id.is_empty() && !out.iter().any(|u| u.id == f.m.from.id) {
                        out.push(f.m.from.clone());
                    }
                }
            }
        }
        out.retain(|u| !u.name.is_empty());
        out
    }

    /// Tab-complete "@prefix" to a participant's full name.
    fn complete_mention(&mut self) -> bool {
        let before = &self.compose.text[..self.compose.cursor];
        let Some(at) = before.rfind('@') else { return false };
        let token = before[at + 1..].to_lowercase();
        if token.is_empty() || token.contains('\n') || token.len() > 40 {
            return false;
        }
        let Some(u) = self.mention_candidates().into_iter().find(|u| u.name.to_lowercase().starts_with(&token) && u.name.len() > token.len()) else {
            return false;
        };
        let cursor = self.compose.cursor;
        self.compose.text.replace_range(at + 1..cursor, &format!("{} ", u.name));
        self.compose.cursor = at + 1 + u.name.len() + 1;
        true
    }

    fn selected(&self) -> Option<(Message, MsgId)> {
        let conv = self.conv()?;
        let flat = conv.flat();
        let f = flat.get(conv.sel?)?;
        let mut m = f.m.clone();
        m.replies.clear();
        Some((m, MsgId { id: f.m.id.clone(), parent: f.parent.map(str::to_string) }))
    }

    fn send(&mut self) {
        let text = self.compose.text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(target) = self.conv().map(|c| c.target.clone()) else { return };
        let candidates = self.mention_candidates();
        self.compose.take();
        if let Some(id) = self.editing.take() {
            if let Some(c) = self.conv_mut() {
                for m in c.msgs.iter_mut().flat_map(|m| std::iter::once(&mut *m)) {
                    if m.id == id.id {
                        m.body = text.clone();
                        m.edited = true;
                    }
                    for r in m.replies.iter_mut().filter(|r| r.id == id.id) {
                        r.body = text.clone();
                        r.edited = true;
                    }
                }
            }
            let t = target.clone();
            self.act("Couldn't edit the message", None, Some(target), false, move |b| async move { b.edit(&t, &id, &text).await });
            return;
        }
        let reply = self.reply.take();
        let is_channel = matches!(target, ConvRef::Channel { .. });
        let thread = if is_channel { reply.as_ref().map(|r| r.id.clone()) } else { None };
        let quote = if is_channel { None } else { reply.map(|r| (r.who, r.text)) };
        let mentions: Vec<User> = candidates.into_iter().filter(|u| text.contains(&format!("@{}", u.name))).collect();
        // Optimistic echo, replaced by the server copy on the next refresh.
        self.pending_seq += 1;
        let body = match &quote {
            Some((who, q)) => format!("▎ {who}\n▎ {}\n{text}", q.lines().next().unwrap_or("")),
            None => text.clone(),
        };
        let echo = Message {
            id: format!("pending-{}", self.pending_seq),
            from: self.me.clone(),
            subject: None,
            body,
            created: Utc::now(),
            edited: false,
            deleted: false,
            system: false,
            reactions: vec![],
            attachments: vec![],
            mentions: vec![],
            replies: vec![],
        };
        if let Some(c) = self.conv_mut() {
            match thread.as_ref().and_then(|t| c.msgs.iter_mut().find(|m| &m.id == t)) {
                Some(root) => root.replies.push(echo),
                None => c.msgs.push(echo),
            }
            c.scroll = 0;
            c.sel = None;
        }
        let out = Outgoing { text, mentions, quote, subject: None };
        let t = target.clone();
        self.act("Couldn't send the message", None, Some(target), false, move |b| async move { b.send(&t, thread, out).await });
    }

    fn start_reply(&mut self) {
        let Some((m, id)) = self.selected() else { return };
        if m.system || m.deleted {
            return;
        }
        let is_channel = self.conv().is_some_and(|c| c.is_channel());
        self.editing = None;
        self.reply = Some(if is_channel {
            // Replies always attach to the root post of the thread.
            let root = id.parent.clone().unwrap_or(id.id.clone());
            let who = self.conv().and_then(|c| c.msgs.iter().find(|x| x.id == root).map(|x| x.from.name.clone())).unwrap_or(m.from.name.clone());
            ReplyCtx { id: root, who, text: m.body }
        } else {
            ReplyCtx { id: id.id, who: m.from.name, text: m.body }
        });
        self.focus = Focus::Compose;
    }

    fn start_edit(&mut self) {
        let Some((m, id)) = self.selected() else { return };
        if m.from.id != self.me.id || m.deleted || m.system {
            self.error("You can only edit your own messages");
            return;
        }
        // Don't carry a quoted block into the editor.
        let body: Vec<&str> = m.body.lines().skip_while(|l| l.starts_with('▎')).collect();
        self.compose.set(&body.join("\n"));
        self.reply = None;
        self.editing = Some(id);
        self.focus = Focus::Compose;
    }

    fn ask_delete(&mut self) {
        let Some((m, _)) = self.selected() else { return };
        if m.from.id != self.me.id || m.deleted || m.system {
            self.error("You can only delete your own messages");
            return;
        }
        self.modal = Modal::ConfirmDelete;
    }

    fn delete_selected(&mut self) {
        let Some((_, id)) = self.selected() else { return };
        let Some(target) = self.conv().map(|c| c.target.clone()) else { return };
        let t = target.clone();
        self.act("Couldn't delete the message", Some("Message deleted".into()), Some(target), false, move |b| async move { b.delete(&t, &id).await });
    }

    fn react(&mut self, emoji: &'static str) {
        let Some((m, id)) = self.selected() else { return };
        let Some(target) = self.conv().map(|c| c.target.clone()) else { return };
        let add = !m.reactions.iter().any(|r| r.user_id == self.me.id && r.emoji == emoji);
        let me = self.me.id.clone();
        if let Some(c) = self.conv_mut() {
            for root in &mut c.msgs {
                for x in std::iter::once(&mut *root).filter(|x| x.id == id.id) {
                    x.reactions.retain(|r| !(r.user_id == me && r.emoji == emoji));
                    if add {
                        x.reactions.push(Reaction { emoji: emoji.into(), user_id: me.clone() });
                    }
                }
                for x in root.replies.iter_mut().filter(|x| x.id == id.id) {
                    x.reactions.retain(|r| !(r.user_id == me && r.emoji == emoji));
                    if add {
                        x.reactions.push(Reaction { emoji: emoji.into(), user_id: me.clone() });
                    }
                }
            }
        }
        let t = target.clone();
        self.act("Couldn't update the reaction", None, Some(target), false, move |b| async move { b.react(&t, &id, emoji, add).await });
    }

    fn open_selected_link(&mut self) {
        let Some((m, _)) = self.selected() else { return };
        if let Some(a) = m.attachments.iter().find(|a| !a.url.is_empty()) {
            if a.is_image() && a.url.contains("graph.microsoft.com") && !self.demo {
                // Teams-hosted images need the auth token, so fetch them and open locally.
                return self.fetch_and_open_image(a.url.clone(), a.is_gif());
            }
            let url = a.url.clone();
            return self.open_url(&url);
        }
        match first_url(&m.body) {
            Some(u) => self.open_url(&u),
            None => self.toast("No link, image or attachment in this message"),
        }
    }

    /// Download a Graph-hosted image to a temp file and open it in the OS viewer.
    fn fetch_and_open_image(&mut self, url: String, gif: bool) {
        self.img_seq += 1;
        let mut dest = std::env::temp_dir();
        dest.push(format!("teamstui-image-{}.{}", self.img_seq, if gif { "gif" } else { "png" }));
        self.toast("Opening image\u{2026}");
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let ev = match backend.fetch(&url, dest.clone()).await {
                Ok(()) => match open::that_detached(&dest) {
                    Ok(()) => Ev::Done { toast: Some("Opened image in your viewer".into()), refresh: None, calendar: false },
                    Err(_) => Ev::Error(format!("Image saved to {} but no viewer could open it", dest.display())),
                },
                Err(e) => Ev::Error(format!("Couldn't load the image: {e}")),
            };
            let _ = tx.send(ev);
        });
    }

    /// Open the in-terminal image viewer for the selected message's image.
    fn view_image(&mut self) {
        let Some((m, _)) = self.selected() else { return };
        let Some(a) = m.attachments.iter().find(|a| a.is_image()) else {
            return self.toast("No image in this message");
        };
        let (url, gif, title) = (a.url.clone(), a.is_gif(), a.name.clone());
        self.modal = Modal::Image { title, url: url.clone(), gif, img: None, error: None };
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let key = url.clone();
        tokio::spawn(async move {
            let img = match backend.fetch_bytes(&key).await {
                Ok(bytes) => tokio::task::spawn_blocking(move || image::load_from_memory(&bytes).map_err(|e| e.to_string()))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(Ev::ImageLoaded { url, img });
        });
    }

    fn open_image_external(&mut self, url: String, gif: bool) {
        if url.contains("graph.microsoft.com") && !self.demo {
            self.fetch_and_open_image(url, gif);
        } else {
            self.open_url(&url);
        }
    }

    // ---- events ----------------------------------------------------------

    pub fn on_event(&mut self, ev: Ev) {
        match ev {
            Ev::Input(Event::Key(k)) => self.on_key(k),
            Ev::Input(Event::Mouse(m)) => self.on_mouse(m),
            Ev::Input(Event::Paste(s)) => self.on_paste(&s),
            Ev::Input(_) | Ev::DeviceCode(_) | Ev::LoginDone(_) => {}
            Ev::Tick => self.tick(),
            Ev::Me(u) => {
                self.me = u;
                self.rebuild_activity();
                self.load_presence();
                // Titles depend on who "me" is.
                let id = self.chat_conv.as_ref().and_then(|c| match &c.target {
                    ConvRef::Chat(id) => Some(id.clone()),
                    _ => None,
                });
                if let (Some(id), Some(c)) = (id, self.chat_conv.as_mut()) {
                    if let Some(chat) = self.chats.iter().find(|x| x.id == id) {
                        c.title = chat.title(&self.me.id);
                    }
                }
            }
            Ev::Chats(new) => self.on_chats(new),
            Ev::Messages { conv, page, older } => {
                if !older {
                    self.conv_inflight = false;
                }
                let mut title = None;
                for c in [&mut self.chat_conv, &mut self.chan_conv].into_iter().flatten() {
                    if c.target == conv {
                        c.apply(page.clone(), older);
                        title = Some(c.title.clone());
                    }
                }
                if let Some(t) = title {
                    self.collect_mentions(&conv, &t);
                }
            }
            Ev::Teams(t) => {
                self.teams = t;
                self.teams_loaded = true;
                self.team_sel = self.team_sel.min(self.team_rows().len().saturating_sub(1));
            }
            Ev::Events(week, list) => {
                if week == self.week_start {
                    let keep = self.events.get(self.ev_sel).map(|e| e.id.clone());
                    let first = !self.events_loaded;
                    self.events = list;
                    self.events_loaded = true;
                    self.ev_sel = match keep.and_then(|id| self.events.iter().position(|e| e.id == id)) {
                        Some(i) => i,
                        // Start on the next upcoming event.
                        None if first => self.events.iter().position(|e| e.end > Local::now()).unwrap_or(0),
                        None => self.ev_sel.min(self.events.len().saturating_sub(1)),
                    };
                    if let Modal::Event(i) = self.modal {
                        if i >= self.events.len() {
                            self.modal = Modal::None;
                        }
                    }
                }
            }
            Ev::Presence(p) => {
                if let Some(a) = p.get(&self.me.id) {
                    self.my_presence = *a;
                }
                self.presence.extend(p);
            }
            Ev::People(q, users) => {
                if let Modal::NewChat { input, results, sel, picked } = &mut self.modal {
                    if input.text.trim() == q {
                        for u in users {
                            if !results.iter().any(|r| r.id == u.id) && !picked.iter().any(|p| p.id == u.id) {
                                results.push(u);
                            }
                        }
                        *sel = (*sel).min(results.len().saturating_sub(1));
                    }
                }
            }
            Ev::SearchHits(q, people, hits) => {
                if let Modal::Search { input, results, loading, .. } = &mut self.modal {
                    if input.text.trim() == q {
                        *loading = false;
                        for u in people {
                            if !results.iter().any(|r| matches!(r, SearchItem::Person(p) if p.id == u.id)) {
                                results.push(SearchItem::Person(u));
                            }
                        }
                        results.extend(hits.into_iter().map(SearchItem::Message));
                    }
                }
            }
            Ev::ChatCreated(chat) => {
                let id = chat.id.clone();
                if !self.chats.iter().any(|c| c.id == id) {
                    self.chats.insert(0, chat);
                }
                self.toast = None;
                self.open_chat_id(&id);
            }
            Ev::Files(f) => {
                self.files = f;
                self.files_loaded = true;
                self.file_sel = self.file_sel.min(self.files.len().saturating_sub(1));
            }
            Ev::ImageLoaded { url, img } => {
                if let Modal::Image { url: u, img: slot, error, .. } = &mut self.modal {
                    if *u == url {
                        match img {
                            Ok(i) => *slot = Some(i),
                            Err(e) => *error = Some(e),
                        }
                    }
                }
            }
            Ev::Done { toast, refresh, calendar } => {
                if let Some(t) = toast {
                    self.toast(t);
                }
                if let Some(conv) = refresh {
                    if matches!(conv, ConvRef::Chat(_)) {
                        self.load_chats();
                    }
                    self.load_messages(conv, None);
                }
                if calendar {
                    self.load_events();
                }
            }
            Ev::Error(e) => {
                // Back off so a persistent failure (e.g. a missing permission) doesn't spam.
                let later = Instant::now() + Duration::from_secs(20);
                self.last_chats = later;
                self.last_conv = later;
                self.conv_inflight = false;
                self.chats_inflight = false;
                self.files_loaded = true;
                for c in [&mut self.chat_conv, &mut self.chan_conv].into_iter().flatten() {
                    c.loading = false;
                    c.loading_older = false;
                }
                if let Modal::Search { loading, .. } = &mut self.modal {
                    *loading = false;
                }
                self.error(e);
            }
        }
    }

    fn on_chats(&mut self, new: Vec<Chat>) {
        self.chats_inflight = false;
        let sel_id = self.chats.get(self.chat_sel).map(|c| c.id.clone());
        let open_id = match (&self.view, self.chat_conv.as_ref().map(|c| &c.target)) {
            (View::Chat, Some(ConvRef::Chat(id))) => Some(id.clone()),
            _ => None,
        };
        if self.chats_loaded {
            let mut note = None;
            for c in &new {
                let newer = self.chats.iter().find(|o| o.id == c.id).map_or(true, |o| c.last_time > o.last_time);
                if newer && c.unread && c.preview_from.id != self.me.id && open_id.as_ref() != Some(&c.id) {
                    note = Some(format!("{}: {}", c.preview_from.name, c.preview));
                }
            }
            if let Some(n) = note {
                self.toast(n);
                if self.bell {
                    use std::io::Write;
                    let _ = std::io::stdout().write_all(b"\x07").and_then(|_| std::io::stdout().flush());
                }
            }
        }
        let first = !self.chats_loaded;
        self.chats = new;
        self.chats_loaded = true;
        self.chat_sel = sel_id.and_then(|id| self.chats.iter().position(|c| c.id == id)).unwrap_or(self.chat_sel.min(self.chats.len().saturating_sub(1)));
        if first && self.chat_conv.is_none() && self.view == View::Chat && matches!(self.modal, Modal::None) {
            if let Some(id) = self.chats.first().map(|c| c.id.clone()) {
                self.open_chat_id(&id);
            }
        } else {
            self.mark_open_chat_read();
        }
        self.rebuild_activity();
        if first {
            self.load_presence();
        }
    }

    fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
        if self.toast.as_ref().is_some_and(|(_, t, err)| t.elapsed() > Duration::from_secs(if *err { 8 } else { 4 })) {
            self.toast = None;
        }
        if self.last_chats.elapsed() > Duration::from_secs(8) {
            self.load_chats();
        }
        if self.last_presence.elapsed() > Duration::from_secs(30) {
            self.load_presence();
        }
        if self.last_events.elapsed() > Duration::from_secs(120) {
            self.load_events();
        }
        if !self.conv_inflight && self.last_conv.elapsed() > Duration::from_secs(4) {
            if let Some(t) = self.conv().map(|c| c.target.clone()) {
                self.load_messages(t, None);
            }
        }
        let older = self.conv_mut().and_then(|c| {
            let go = c.want_older && !c.loading_older && c.cursor.is_some();
            c.want_older = false;
            go.then(|| {
                c.loading_older = true;
                (c.target.clone(), c.cursor.clone())
            })
        });
        if let Some((t, cursor)) = older {
            self.load_messages(t, cursor);
        }
        if self.search_due.is_some_and(|t| t.elapsed() > Duration::from_millis(350)) {
            self.search_due = None;
            self.remote_search();
        }
    }

    fn remote_search(&mut self) {
        match &mut self.modal {
            Modal::Search { input, loading, .. } => {
                let q = input.text.trim().to_string();
                if q.chars().count() < 2 {
                    return;
                }
                *loading = true;
                self.spawn(|b| async move {
                    let (people, hits) = futures::join!(b.search_people(&q), b.search_messages(&q));
                    match (people, hits) {
                        (Err(e), Err(_)) => Ev::Error(format!("Search failed: {e}")),
                        (p, h) => Ev::SearchHits(q, p.unwrap_or_default(), h.unwrap_or_default()),
                    }
                });
            }
            Modal::NewChat { input, .. } => {
                let q = input.text.trim().to_string();
                if q.chars().count() < 2 {
                    return;
                }
                self.spawn(|b| async move {
                    match b.search_people(&q).await {
                        Ok(p) => Ev::People(q, p),
                        Err(e) => Ev::Error(format!("People search failed: {e}")),
                    }
                });
            }
            _ => {}
        }
    }

    fn local_search(&mut self) {
        let Modal::Search { input, .. } = &self.modal else { return };
        let q = input.text.trim().to_lowercase();
        let mut found = vec![];
        if !q.is_empty() {
            for c in &self.chats {
                let title = c.title(&self.me.id);
                if title.to_lowercase().contains(&q) {
                    found.push(SearchItem::Chat { id: c.id.clone(), title });
                }
            }
            for (ti, t) in self.teams.iter().enumerate() {
                for (ci, ch) in t.channels.iter().enumerate() {
                    if ch.name.to_lowercase().contains(&q) || t.name.to_lowercase().contains(&q) {
                        found.push(SearchItem::Channel { team: ti, channel: ci, title: format!("{} › {}", t.name, ch.name) });
                    }
                }
            }
        }
        if let Modal::Search { results, sel, .. } = &mut self.modal {
            *results = found;
            *sel = 0;
        }
        self.search_due = Some(Instant::now());
    }

    fn local_people(&mut self) {
        let contacts = self.contacts();
        if let Modal::NewChat { input, results, sel, picked } = &mut self.modal {
            let q = input.text.trim().to_lowercase();
            *results = if q.is_empty() {
                vec![]
            } else {
                contacts.into_iter().filter(|u| (u.name.to_lowercase().contains(&q) || u.email.to_lowercase().contains(&q)) && !picked.iter().any(|p| p.id == u.id)).collect()
            };
            *sel = 0;
        }
        self.search_due = Some(Instant::now());
    }

    fn on_paste(&mut self, s: &str) {
        match &mut self.modal {
            Modal::None => {
                if self.conv().is_some() {
                    self.focus = Focus::Compose;
                    self.compose.insert_str(s);
                }
            }
            Modal::Search { input, .. } => {
                input.insert_str(&s.replace('\n', " "));
                self.local_search();
            }
            Modal::NewChat { input, .. } => {
                input.insert_str(&s.replace('\n', " "));
                self.local_people();
            }
            Modal::StatusMsg { input } => input.insert_str(&s.replace('\n', " ")),
            Modal::NewMeeting { fields, focus } => fields[*focus].insert_str(&s.replace('\n', " ")),
            _ => {}
        }
    }

    // ---- keyboard --------------------------------------------------------

    fn open_search(&mut self) {
        self.modal = Modal::Search { input: Input::default(), results: vec![], sel: 0, loading: false };
    }

    fn new_item(&mut self) {
        if self.view == View::Calendar {
            let now = Local::now();
            let next = now + CDuration::minutes(30 - (now.minute() % 30) as i64);
            let fields = vec![Input::default(), Input::with(&next.format("%Y-%m-%d").to_string()), Input::with(&next.format("%H:%M").to_string()), Input::with("30"), Input::default()];
            self.modal = Modal::NewMeeting { fields, focus: 0 };
        } else {
            self.modal = Modal::NewChat { input: Input::default(), results: vec![], sel: 0, picked: vec![] };
        }
    }

    fn refresh(&mut self) {
        self.chats_inflight = false;
        self.load_chats();
        self.load_teams();
        self.load_events();
        self.load_presence();
        if self.view == View::Files {
            self.load_files();
        }
        if let Some(t) = self.conv().map(|c| c.target.clone()) {
            self.load_messages(t, None);
        }
        self.toast("Refreshing…");
    }

    fn cycle_focus(&mut self, back: bool) {
        if self.conv().is_none() {
            self.focus = Focus::List;
            return;
        }
        let order = [Focus::List, Focus::Messages, Focus::Compose];
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = order[(i + if back { 2 } else { 1 }) % 3];
        if self.focus == Focus::Messages {
            self.select_last_if_none();
        }
    }

    fn select_last_if_none(&mut self) {
        if let Some(c) = self.conv_mut() {
            let n = c.flat().len();
            if c.sel.is_none() && n > 0 {
                c.sel = Some(n - 1);
                c.follow = true;
            }
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        if ctrl && matches!(k.code, KeyCode::Char('q') | KeyCode::Char('c')) {
            self.quit = true;
            return;
        }
        if !matches!(self.modal, Modal::None) {
            self.modal_key(k);
            return;
        }
        match k.code {
            KeyCode::Char(c @ '1'..='6') if alt || ctrl => return self.set_view(VIEWS[c as usize - '1' as usize]),
            KeyCode::Char('e') if ctrl => return self.open_search(),
            KeyCode::Char('n') if ctrl => return self.new_item(),
            KeyCode::Char('p') if ctrl => return self.modal = Modal::Presence { sel: 0 },
            KeyCode::Char('r') if ctrl => return self.refresh(),
            KeyCode::F(1) => return self.modal = Modal::Help,
            KeyCode::Tab => {
                if self.focus == Focus::Compose && self.complete_mention() {
                    return;
                }
                return self.cycle_focus(false);
            }
            KeyCode::BackTab => return self.cycle_focus(true),
            _ => {}
        }
        if self.conv().is_none() {
            self.focus = Focus::List;
        }
        match self.focus {
            Focus::List => self.list_key(k),
            Focus::Messages => self.messages_key(k),
            Focus::Compose => self.compose_key(k),
        }
    }

    fn list_key(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1),
            KeyCode::PageUp => self.move_sel(-10),
            KeyCode::PageDown => self.move_sel(10),
            KeyCode::Home | KeyCode::Char('g') => self.move_sel(-100_000),
            KeyCode::End | KeyCode::Char('G') => self.move_sel(100_000),
            KeyCode::Enter => self.activate_list(),
            KeyCode::Char('?') => self.modal = Modal::Help,
            KeyCode::Char('/') => self.open_search(),
            KeyCode::Char('n') => self.new_item(),
            KeyCode::Char('q') => self.quit = true,
            _ => match self.view {
                View::Teams => {
                    if let Some(TeamRow::Team(ti)) = self.team_rows().get(self.team_sel).copied() {
                        let id = self.teams[ti].id.clone();
                        match k.code {
                            KeyCode::Left | KeyCode::Char('h') => {
                                self.collapsed.insert(id);
                            }
                            KeyCode::Right | KeyCode::Char('l') => {
                                self.collapsed.remove(&id);
                            }
                            _ => {}
                        }
                    } else if matches!(k.code, KeyCode::Right | KeyCode::Char('l')) && self.conv().is_some() {
                        self.focus = Focus::Compose;
                    }
                }
                View::Calendar => match k.code {
                    KeyCode::Left | KeyCode::Char('h') => self.shift_week(-1),
                    KeyCode::Right | KeyCode::Char('l') => self.shift_week(1),
                    KeyCode::Char('t') => self.shift_week(0),
                    KeyCode::Char('J') => {
                        if let Some(url) = self.events.get(self.ev_sel).and_then(|e| e.join_url.clone()) {
                            self.open_url(&url);
                        }
                    }
                    _ => {}
                },
                View::Calls => match k.code {
                    KeyCode::Char('c') => self.start_call(false),
                    KeyCode::Char('v') => self.start_call(true),
                    KeyCode::Char('m') => {
                        if let Some(u) = self.contacts().get(self.call_sel).cloned() {
                            self.open_person(u);
                        }
                    }
                    _ => {}
                },
                View::Files => match k.code {
                    KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => self.files_up(),
                    KeyCode::Right | KeyCode::Char('l') => self.activate_list(),
                    KeyCode::Char('d') => self.download_selected(),
                    _ => {}
                },
                View::Chat => {
                    if matches!(k.code, KeyCode::Right | KeyCode::Char('l')) && self.conv().is_some() {
                        self.focus = Focus::Compose;
                    }
                }
                View::Activity => {}
            },
        }
    }

    fn download_selected(&mut self) {
        let Some(item) = self.files.get(self.file_sel).cloned() else { return };
        if item.folder {
            self.error("Folders can't be downloaded — open it and pick a file");
            return;
        }
        let dir = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| ".".into());
        let name = item.name.replace(['/', '\\'], "_");
        let dest = dir.join(&name);
        self.toast(format!("Downloading {name}…"));
        let shown = dest.display().to_string();
        self.act("Download failed", Some(format!("Saved to {shown}")), None, false, move |b| async move { b.download(&item, dest).await });
    }

    fn scroll_conv(&mut self, delta: i64) {
        if let Some(c) = self.conv_mut() {
            c.scroll = (c.scroll as i64 + delta).max(0) as usize;
            c.follow = false;
        }
    }

    fn messages_key(&mut self, k: KeyEvent) {
        let n = self.conv().map_or(0, |c| c.flat().len());
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(c) = self.conv_mut() {
                    match c.sel {
                        Some(0) => c.want_older = true,
                        Some(i) => c.sel = Some(i - 1),
                        None if n > 0 => c.sel = Some(n - 1),
                        None => {}
                    }
                    c.follow = true;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let at_end = self.conv().is_some_and(|c| c.sel.map_or(true, |i| i + 1 >= n));
                if at_end {
                    if let Some(c) = self.conv_mut() {
                        c.sel = None;
                        c.scroll = 0;
                    }
                    self.focus = Focus::Compose;
                } else if let Some(c) = self.conv_mut() {
                    c.sel = c.sel.map(|i| i + 1);
                    c.follow = true;
                }
            }
            KeyCode::Home | KeyCode::Char('g') => {
                if let Some(c) = self.conv_mut() {
                    c.sel = (n > 0).then_some(0);
                    c.follow = true;
                    c.want_older = true;
                }
            }
            KeyCode::End | KeyCode::Char('G') => {
                if let Some(c) = self.conv_mut() {
                    c.sel = n.checked_sub(1);
                    c.follow = true;
                }
            }
            KeyCode::PageUp => self.scroll_conv(10),
            KeyCode::PageDown => self.scroll_conv(-10),
            KeyCode::Char('r') => {
                if self.selected().is_some_and(|(m, _)| !m.system && !m.deleted) {
                    self.modal = Modal::React { sel: 0 };
                }
            }
            KeyCode::Char('e') => self.start_edit(),
            KeyCode::Char('d') | KeyCode::Delete => self.ask_delete(),
            KeyCode::Enter | KeyCode::Char('y') => self.start_reply(),
            KeyCode::Char('o') => self.open_selected_link(),
            KeyCode::Char('v') => self.view_image(),
            KeyCode::Char('c') => self.start_call(false),
            KeyCode::Char('?') => self.modal = Modal::Help,
            KeyCode::Char('/') => self.open_search(),
            KeyCode::Left | KeyCode::Char('h') => self.focus = Focus::List,
            KeyCode::Esc | KeyCode::Char('i') => {
                if let Some(c) = self.conv_mut() {
                    c.sel = None;
                    c.scroll = 0;
                }
                self.focus = Focus::Compose;
            }
            _ => {}
        }
    }

    fn compose_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Enter if alt || shift || ctrl => self.compose.insert('\n'),
            KeyCode::Char('j') if ctrl => self.compose.insert('\n'),
            KeyCode::Enter => self.send(),
            KeyCode::Esc => {
                if self.editing.is_some() {
                    self.editing = None;
                    self.compose.take();
                } else if self.reply.is_some() {
                    self.reply = None;
                } else {
                    self.focus = Focus::List;
                }
            }
            KeyCode::Up if !self.compose.text.contains('\n') => {
                self.focus = Focus::Messages;
                self.select_last_if_none();
            }
            KeyCode::PageUp => self.scroll_conv(10),
            KeyCode::PageDown => self.scroll_conv(-10),
            _ => {
                self.compose.handle_key(k);
            }
        }
    }

    fn submit_meeting(&mut self) {
        let Modal::NewMeeting { fields, .. } = &self.modal else { return };
        let get = |i: usize| fields[i].text.trim().to_string();
        let subject = if get(0).is_empty() { "New meeting".to_string() } else { get(0) };
        let Ok(date) = NaiveDate::parse_from_str(&get(1), "%Y-%m-%d") else { return self.error("Date must look like 2026-11-20") };
        let Ok(time) = NaiveTime::parse_from_str(&get(2), "%H:%M") else { return self.error("Start time must look like 14:30") };
        let Some(mins) = get(3).parse::<i64>().ok().filter(|m| (5..=24 * 60).contains(m)) else { return self.error("Duration must be between 5 and 1440 minutes") };
        let Some(start) = date.and_time(time).and_local_timezone(Local).earliest() else { return self.error("That local time doesn't exist") };
        let attendees: Vec<String> = get(4).split([',', ';', ' ']).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        if let Some(bad) = attendees.iter().find(|a| !a.contains('@')) {
            return self.error(format!("'{bad}' is not an email address"));
        }
        let start = start.with_timezone(&Utc);
        let ev = NewEvent { subject, start, end: start + CDuration::minutes(mins), attendees };
        self.modal = Modal::None;
        self.week_start = monday(date);
        self.act("Couldn't schedule the meeting", Some("Meeting scheduled".into()), None, true, move |b| async move { b.create_event(ev).await });
    }

    fn apply_presence_choice(&mut self, sel: usize) {
        self.modal = Modal::None;
        let n = Availability::SETTABLE.len();
        if sel < n {
            let a = Availability::SETTABLE[sel];
            self.my_presence = a;
            self.act("Couldn't set your status", Some(format!("Status set to {}", a.label())), None, false, move |b| async move { b.set_presence(Some(a)).await });
        } else if sel == n {
            self.act("Couldn't reset your status", Some("Status reset".into()), None, false, |b| async move { b.set_presence(None).await });
            self.last_presence = Instant::now() - Duration::from_secs(27);
        } else if sel == n + 1 {
            self.modal = Modal::StatusMsg { input: Input::with(&self.status_msg) };
        } else {
            if !self.demo {
                let _ = auth::logout();
            }
            self.exit_msg = Some("Signed out of TeamsTUI.".into());
            self.quit = true;
        }
    }

    fn activate_search(&mut self, item: SearchItem) {
        self.modal = Modal::None;
        match item {
            SearchItem::Chat { id, .. } => self.open_chat_id(&id),
            SearchItem::Channel { team, channel, .. } => self.open_channel(team, channel),
            SearchItem::Person(u) => self.open_person(u),
            SearchItem::Message(h) => match h.conv {
                Some(c) => self.open_target(&c),
                None => self.toast("Can't locate that message's conversation"),
            },
        }
    }

    fn pick_person(&mut self) {
        let mut create = None;
        if let Modal::NewChat { input, results, sel, picked } = &mut self.modal {
            if let Some(u) = results.get(*sel).cloned() {
                picked.push(u);
                input.take();
                results.clear();
                *sel = 0;
            } else if input.text.trim().is_empty() && !picked.is_empty() {
                create = Some(picked.clone());
            }
        }
        if let Some(users) = create {
            self.modal = Modal::None;
            if users.len() == 1 {
                self.open_person(users.into_iter().next().unwrap());
            } else {
                self.create_chat(users);
            }
        }
    }

    fn modal_key(&mut self, k: KeyEvent) {
        if k.code == KeyCode::Esc {
            self.modal = Modal::None;
            return;
        }
        let step = |sel: &mut usize, len: usize, down: bool| {
            if len > 0 {
                *sel = if down { (*sel + 1) % len } else { (*sel + len - 1) % len };
            }
        };
        match &mut self.modal {
            Modal::None => {}
            Modal::Help => self.modal = Modal::None,
            Modal::Search { input, results, sel, .. } => match k.code {
                KeyCode::Up => step(sel, results.len(), false),
                KeyCode::Down | KeyCode::Tab => step(sel, results.len(), true),
                KeyCode::Enter => {
                    if let Some(item) = results.get(*sel).cloned() {
                        self.activate_search(item);
                    } else {
                        self.search_due = None;
                        self.remote_search();
                    }
                }
                _ => {
                    let before = input.text.clone();
                    input.handle_key(k);
                    if input.text != before {
                        self.local_search();
                    }
                }
            },
            Modal::NewChat { input, results, sel, picked } => match k.code {
                KeyCode::Up => step(sel, results.len(), false),
                KeyCode::Down | KeyCode::Tab => step(sel, results.len(), true),
                KeyCode::Enter => self.pick_person(),
                KeyCode::Backspace if input.text.is_empty() => {
                    picked.pop();
                }
                _ => {
                    let before = input.text.clone();
                    input.handle_key(k);
                    if input.text != before {
                        self.local_people();
                    }
                }
            },
            Modal::React { sel } => match k.code {
                KeyCode::Left | KeyCode::Char('h') => step(sel, REACTIONS.len(), false),
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => step(sel, REACTIONS.len(), true),
                KeyCode::Char(c @ '1'..='6') => {
                    self.modal = Modal::None;
                    self.react(REACTIONS[c as usize - '1' as usize]);
                }
                KeyCode::Enter => {
                    let e = REACTIONS[*sel];
                    self.modal = Modal::None;
                    self.react(e);
                }
                _ => {}
            },
            Modal::Presence { sel } => {
                let len = Availability::SETTABLE.len() + PRESENCE_EXTRA.len();
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => step(sel, len, false),
                    KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => step(sel, len, true),
                    KeyCode::Enter => {
                        let s = *sel;
                        self.apply_presence_choice(s);
                    }
                    _ => {}
                }
            }
            Modal::StatusMsg { input } => match k.code {
                KeyCode::Enter => {
                    let text = input.text.trim().to_string();
                    self.status_msg = text.clone();
                    self.modal = Modal::None;
                    let ok = if text.is_empty() { "Status message cleared" } else { "Status message set" };
                    self.act("Couldn't set the status message", Some(ok.into()), None, false, move |b| async move { b.set_status_message(&text).await });
                }
                _ => {
                    input.handle_key(k);
                }
            },
            Modal::ConfirmDelete => match k.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('d') => {
                    self.modal = Modal::None;
                    self.delete_selected();
                }
                _ => self.modal = Modal::None,
            },
            Modal::Event(i) => {
                let Some(ev) = self.events.get(*i).cloned() else {
                    self.modal = Modal::None;
                    return;
                };
                let rsvp = |app: &mut App, r: Rsvp, label: &str| {
                    let id = ev.id.clone();
                    app.modal = Modal::None;
                    app.act("Couldn't respond", Some(format!("{label}: {}", ev.subject)), None, true, move |b| async move { b.rsvp(&id, r).await });
                };
                match k.code {
                    KeyCode::Char('j') | KeyCode::Char('J') => match &ev.join_url {
                        Some(url) => {
                            let url = url.clone();
                            self.modal = Modal::None;
                            if self.demo {
                                self.toast("Demo mode — would join the meeting in Teams");
                            } else {
                                self.open_url(&url);
                            }
                        }
                        None => self.toast("This event has no online meeting link"),
                    },
                    KeyCode::Char('o') => {
                        self.modal = Modal::None;
                        self.open_url(&ev.web_link);
                    }
                    KeyCode::Char('a') => rsvp(self, Rsvp::Accept, "Accepted"),
                    KeyCode::Char('t') => rsvp(self, Rsvp::Tentative, "Tentative"),
                    KeyCode::Char('d') => rsvp(self, Rsvp::Decline, "Declined"),
                    KeyCode::Enter | KeyCode::Char('q') => self.modal = Modal::None,
                    _ => {}
                }
            }
            Modal::Image { url, gif, .. } => {
                if let KeyCode::Char('o') = k.code {
                    let (u, g) = (url.clone(), *gif);
                    self.modal = Modal::None;
                    self.open_image_external(u, g);
                }
            }
            Modal::NewMeeting { fields, focus } => match k.code {
                KeyCode::Tab | KeyCode::Down => *focus = (*focus + 1) % fields.len(),
                KeyCode::BackTab | KeyCode::Up => *focus = (*focus + fields.len() - 1) % fields.len(),
                KeyCode::Enter => self.submit_meeting(),
                _ => {
                    fields[*focus].handle_key(k);
                }
            },
        }
    }

    // ---- mouse -----------------------------------------------------------

    fn on_mouse(&mut self, m: MouseEvent) {
        let pos = Position::new(m.column, m.row);
        let hit = self.hits.iter().rev().find(|(r, _)| r.contains(pos)).map(|(_, h)| h.clone());
        let Some(hit) = hit else { return };
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => self.click(hit),
            MouseEventKind::ScrollUp => self.wheel(hit, true),
            MouseEventKind::ScrollDown => self.wheel(hit, false),
            _ => {}
        }
    }

    fn wheel(&mut self, hit: Hit, up: bool) {
        match hit {
            Hit::Message(_) | Hit::Messages => self.scroll_conv(if up { 3 } else { -3 }),
            Hit::List(_) | Hit::ListArea | Hit::FileRow(_) | Hit::CalEvent(_) if matches!(self.modal, Modal::None) => self.move_sel(if up { -1 } else { 1 }),
            _ => {}
        }
    }

    fn click(&mut self, hit: Hit) {
        match hit {
            Hit::Backdrop => self.modal = Modal::None,
            Hit::ModalBody => {}
            Hit::ModalItem(i) => {
                match &mut self.modal {
                    Modal::Search { sel, .. } | Modal::NewChat { sel, .. } | Modal::React { sel } | Modal::Presence { sel } => *sel = i,
                    Modal::NewMeeting { focus, .. } => {
                        *focus = i;
                        return;
                    }
                    _ => return,
                }
                self.modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            }
            Hit::Rail(v) => self.set_view(v),
            Hit::Help => self.modal = Modal::Help,
            Hit::Search => self.open_search(),
            Hit::Profile => self.modal = Modal::Presence { sel: 0 },
            Hit::List(i) => {
                self.focus = Focus::List;
                *self.sel_mut() = i;
                self.activate_list();
            }
            Hit::ListArea => self.focus = Focus::List,
            Hit::Message(i) => {
                self.focus = Focus::Messages;
                if let Some(c) = self.conv_mut() {
                    c.sel = Some(i);
                    c.follow = false;
                }
            }
            Hit::Messages => self.focus = Focus::Messages,
            Hit::Compose => self.focus = Focus::Compose,
            Hit::Call(video) => self.start_call(video),
            Hit::NewItem => self.new_item(),
            Hit::CalNav(d) => self.shift_week(d),
            Hit::CalEvent(i) => {
                self.ev_sel = i;
                self.modal = Modal::Event(i);
            }
            Hit::FileRow(i) => {
                if self.file_sel == i {
                    self.activate_list();
                } else {
                    self.file_sel = i;
                }
            }
            Hit::FileUp => self.files_up(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::demo::Demo;
    use ratatui::{backend::TestBackend, Terminal};
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    async fn pump(app: &mut App, rx: &mut UnboundedReceiver<Ev>, ms: u64) {
        let until = tokio::time::Instant::now() + Duration::from_millis(ms);
        while let Ok(Some(ev)) = tokio::time::timeout_at(until, rx.recv()).await {
            app.on_event(ev);
        }
    }

    fn screen(app: &mut App) -> String {
        let mut term = Terminal::new(TestBackend::new(120, 36)).unwrap();
        term.draw(|f| crate::ui::draw(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        buf.content().chunks(120).map(|row| row.iter().map(|c| c.symbol()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
        app.on_key(KeyEvent::new(code, mods));
    }

    fn type_text(app: &mut App, s: &str) {
        for c in s.chars() {
            key(app, KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    #[tokio::test]
    async fn demo_end_to_end() {
        let (tx, mut rx) = unbounded_channel();
        let mut app = App::new(Arc::new(Demo::new()), tx, true);
        app.bell = false;
        app.start();
        pump(&mut app, &mut rx, 200).await;

        // The most recent chat opens automatically and is marked read.
        let s = screen(&mut app);
        assert!(s.contains("Megan Bowen") && s.contains("are we still on"), "{s}");
        assert_eq!(app.focus, Focus::Compose);
        assert!(!app.chats[0].unread);

        // Send a message with a tab-completed mention.
        type_text(&mut app, "on my way @meg");
        key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(app.compose.text, "on my way @Megan Bowen ");
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.conv().unwrap().msgs.last().unwrap().id.starts_with("pending-"));
        pump(&mut app, &mut rx, 200).await;
        let last = app.conv().unwrap().msgs.last().unwrap().clone();
        assert_eq!(last.body, "on my way @Megan Bowen");
        assert_eq!(last.mentions, vec!["u1"]);
        assert!(!last.id.starts_with("pending-"));

        // React to, edit, then delete that message.
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(app.focus, Focus::Messages);
        key(&mut app, KeyCode::Char('r'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);
        type_text(&mut app, "!!");
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        pump(&mut app, &mut rx, 200).await;
        let last = app.conv().unwrap().msgs.last().unwrap().clone();
        assert_eq!(last.body, "on my way @Megan Bowen!!");
        assert!(last.edited);
        assert_eq!(last.reactions.len(), 1);
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('d'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        pump(&mut app, &mut rx, 200).await;
        assert!(app.conv().unwrap().msgs.last().unwrap().deleted);

        // Teams: open a channel and reply in a thread.
        key(&mut app, KeyCode::Char('3'), KeyModifiers::ALT);
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        pump(&mut app, &mut rx, 200).await;
        assert!(screen(&mut app).contains("Release date update"));
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        key(&mut app, KeyCode::Home, KeyModifiers::NONE);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.reply.is_some());
        type_text(&mut app, "ack");
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        pump(&mut app, &mut rx, 200).await;
        assert_eq!(app.conv().unwrap().msgs[0].replies.last().unwrap().body, "ack");

        // Calendar, files, calls, activity and every modal render without panicking.
        for (i, needle) in [(4, "Today"), (6, "My files"), (5, "Megan Bowen"), (1, "Activity")] {
            key(&mut app, KeyCode::Char(char::from_digit(i, 10).unwrap()), KeyModifiers::ALT);
            pump(&mut app, &mut rx, 150).await;
            assert!(screen(&mut app).contains(needle), "view {i}");
        }
        key(&mut app, KeyCode::Char('4'), KeyModifiers::ALT);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(screen(&mut app).contains("Meeting details"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        assert!(screen(&mut app).contains("New meeting"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('e'), KeyModifiers::CONTROL);
        type_text(&mut app, "budget");
        // The remote search is debounced and fires from the tick handler.
        tokio::time::sleep(Duration::from_millis(400)).await;
        app.on_event(Ev::Tick);
        pump(&mut app, &mut rx, 200).await;
        assert!(screen(&mut app).contains("Q4 budget numbers"));
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        pump(&mut app, &mut rx, 150).await;
        assert_eq!(app.view, View::Chat);
        assert_eq!(app.conv().unwrap().title, "Nestor Wilke");
        key(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert!(screen(&mut app).contains("Sign out"));
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.my_presence, Availability::Busy);
        key(&mut app, KeyCode::F(1), KeyModifiers::NONE);
        assert!(screen(&mut app).contains("Keyboard shortcuts"));
    }
}
