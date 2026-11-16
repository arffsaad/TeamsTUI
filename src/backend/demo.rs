//! An in-memory backend with sample data (`--demo`). Nothing leaves the machine.
use super::Backend;
use crate::model::*;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Local, NaiveTime, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

struct Pending {
    due: Instant,
    conv: ConvRef,
    thread: Option<String>,
    msg: Message,
}

struct St {
    me: User,
    people: Vec<User>,
    chats: Vec<Chat>,
    msgs: HashMap<ConvRef, Vec<Message>>,
    teams: Vec<Team>,
    events: Vec<CalEvent>,
    presence: HashMap<String, Availability>,
    pending: Vec<Pending>,
    seq: u64,
}

pub struct Demo {
    st: Mutex<St>,
}

fn person(id: &str, name: &str) -> User {
    User { id: id.into(), name: name.into(), email: format!("{}@contoso.example", name.to_lowercase().replace(' ', ".")) }
}

fn msg(seq: &mut u64, from: &User, mins_ago: i64, text: &str) -> Message {
    *seq += 1;
    Message {
        id: format!("m{seq}"),
        from: from.clone(),
        subject: None,
        body: text.into(),
        created: Utc::now() - Duration::minutes(mins_ago),
        edited: false,
        deleted: false,
        system: false,
        reactions: vec![],
        attachments: vec![],
        mentions: vec![],
        replies: vec![],
    }
}

const REPLIES: [&str; 6] = [
    "Sounds good to me 👍",
    "Let me check and get back to you.",
    "Ha, nice one 😄",
    "Can we talk about it in the standup tomorrow?",
    "Agreed. I'll update the doc.",
    "Thanks for the heads up!",
];

impl Demo {
    pub fn new() -> Self {
        let me = person("me", "Alex Wilber");
        let megan = person("u1", "Megan Bowen");
        let adele = person("u2", "Adele Vance");
        let diego = person("u3", "Diego Siciliani");
        let lynne = person("u4", "Lynne Robbins");
        let nestor = person("u5", "Nestor Wilke");
        let patti = person("u6", "Patti Fernandez");
        let people = vec![megan.clone(), adele.clone(), diego.clone(), lynne.clone(), nestor.clone(), patti.clone()];
        let mut seq = 0u64;
        let mut msgs: HashMap<ConvRef, Vec<Message>> = HashMap::new();
        let mut chats = vec![];
        let mut add_chat = |id: &str, kind: ChatKind, topic: Option<&str>, others: &[&User], m: Vec<Message>, unread: bool| {
            let last = m.last().cloned();
            let mut members: Vec<User> = others.iter().map(|u| (*u).clone()).collect();
            members.push(me.clone());
            chats.push(Chat {
                id: id.into(),
                kind,
                topic: topic.map(str::to_string),
                members,
                preview: last.as_ref().map(|l| l.body.lines().next().unwrap_or("").to_string()).unwrap_or_default(),
                preview_from: last.as_ref().map(|l| l.from.clone()).unwrap_or_default(),
                last_time: last.as_ref().map(|l| l.created),
                unread,
            });
            msgs.insert(ConvRef::Chat(id.into()), m);
        };

        let mut m1 = vec![
            msg(&mut seq, &megan, 60 * 26, "Morning! Did you get a chance to look at the launch checklist?"),
            msg(&mut seq, &me, 60 * 26 - 3, "Yes — two items still open: the pricing page and the release notes."),
            msg(&mut seq, &megan, 60 * 26 - 5, "Great. I can take the release notes."),
            msg(&mut seq, &me, 95, "Pricing page is done, sending it for review now."),
            msg(&mut seq, &megan, 42, "Perfect, thank you! 🎉\nI'll ping legal for the final sign-off."),
            msg(&mut seq, &megan, 4, "@Alex Wilber are we still on for the 2pm design review?"),
        ];
        m1[3].reactions.push(Reaction { emoji: "👍".into(), user_id: "u1".into() });
        m1[4].attachments.push(Attachment { name: "Launch checklist.xlsx".into(), url: "https://contoso.example/files/launch-checklist.xlsx".into() });
        m1[5].mentions.push("me".into());
        add_chat("c1", ChatKind::OneOnOne, None, &[&megan], m1, true);

        let mut m2 = vec![
            msg(&mut seq, &adele, 60 * 5, "Welcome to the launch crew chat everyone 👋"),
            msg(&mut seq, &diego, 60 * 5 - 2, "Thanks Adele! Excited to get this out the door."),
            msg(&mut seq, &lynne, 60 * 4, "Marketing assets are in the shared folder. Feedback welcome before Thursday."),
            msg(&mut seq, &me, 60 * 3, "The demo environment is up. Let me know if anything looks off."),
            msg(&mut seq, &diego, 18, "Demo env looks great. One small thing: the banner still says 'beta'."),
        ];
        m2[3].reactions.push(Reaction { emoji: "❤️".into(), user_id: "u2".into() });
        m2[3].reactions.push(Reaction { emoji: "👍".into(), user_id: "u3".into() });
        m2[3].reactions.push(Reaction { emoji: "👍".into(), user_id: "u4".into() });
        m2[4].attachments.push(Attachment { name: "sunset.jpg".into(), url: "https://contoso.example/images/sunset.jpg".into() });
        let mut gif = msg(&mut seq, &lynne, 10, "Golden hour from the balcony 🌅 (press v to view)");
        gif.attachments.push(Attachment { name: "twilight.png".into(), url: "https://contoso.example/images/twilight.png".into() });
        m2.push(gif);
        add_chat("c2", ChatKind::Group, Some("Launch crew 🚀"), &[&adele, &diego, &lynne], m2, true);

        let m3 = vec![
            msg(&mut seq, &me, 60 * 30, "Hey Nestor, do you have the Q4 budget numbers?"),
            msg(&mut seq, &nestor, 60 * 29, "Yep, uploading the spreadsheet in a minute."),
            msg(&mut seq, &me, 60 * 28, "Thanks a lot!"),
        ];
        add_chat("c3", ChatKind::OneOnOne, None, &[&nestor], m3, false);

        let mut m4 = vec![
            msg(&mut seq, &patti, 60 * 50, "Meeting started"),
            msg(&mut seq, &patti, 60 * 49, "Notes from today: ship date confirmed, QA sign-off needed by Friday."),
            msg(&mut seq, &adele, 60 * 48, "Thanks Patti. Recording is in the channel."),
        ];
        m4[0].system = true;
        add_chat("c4", ChatKind::Meeting, Some("Weekly product sync"), &[&patti, &adele, &megan], m4, false);

        let m5 = vec![msg(&mut seq, &lynne, 60 * 72, "Lunch on Friday?"), msg(&mut seq, &me, 60 * 71, "Count me in 🍜")];
        add_chat("c5", ChatKind::OneOnOne, None, &[&lynne], m5, false);

        let teams = vec![
            Team {
                id: "t1".into(),
                name: "Contoso Product".into(),
                channels: vec![
                    Channel { id: "ch1".into(), name: "General".into(), description: "Announcements and general discussion".into() },
                    Channel { id: "ch2".into(), name: "Design".into(), description: "Design reviews and assets".into() },
                    Channel { id: "ch3".into(), name: "Engineering".into(), description: "Builds, bugs and architecture".into() },
                ],
            },
            Team {
                id: "t2".into(),
                name: "Mark 8 Project Team".into(),
                channels: vec![
                    Channel { id: "ch4".into(), name: "General".into(), description: String::new() },
                    Channel { id: "ch5".into(), name: "Go to Market".into(), description: "Launch planning".into() },
                ],
            },
        ];
        let chan = |t: &str, c: &str| ConvRef::Channel { team: t.into(), channel: c.into() };
        let mut p1 = msg(&mut seq, &adele, 60 * 28, "We're moving the release to the 24th to give QA two more days. Updated timeline is in the Files tab.");
        p1.subject = Some("Release date update".into());
        p1.reactions.push(Reaction { emoji: "👍".into(), user_id: "u1".into() });
        p1.reactions.push(Reaction { emoji: "👍".into(), user_id: "u3".into() });
        p1.replies = vec![
            msg(&mut seq, &diego, 60 * 27, "Works for engineering. We'll use the time for the perf fixes."),
            msg(&mut seq, &megan, 60 * 26, "Updating the comms plan now."),
        ];
        let mut p2 = msg(&mut seq, &patti, 60 * 3, "Reminder: all-hands is tomorrow at 10. Please add your demo topics to the agenda.");
        p2.replies = vec![msg(&mut seq, &me, 60 * 2, "Added the new onboarding flow demo.")];
        let p3 = msg(&mut seq, &lynne, 35, "Who has the latest brand guidelines? The link in the wiki is dead.");
        msgs.insert(chan("t1", "ch1"), vec![p1, p2, p3]);
        let mut d1 = msg(&mut seq, &megan, 60 * 8, "New mockups for the settings page are ready for review.");
        d1.subject = Some("Settings redesign v3".into());
        d1.attachments.push(Attachment { name: "settings-v3.fig".into(), url: "https://contoso.example/files/settings-v3.fig".into() });
        msgs.insert(chan("t1", "ch2"), vec![d1]);
        let mut e1 = msg(&mut seq, &diego, 60 * 6, "Build 4812 is green. Deploying to staging.");
        e1.replies = vec![msg(&mut seq, &nestor, 60 * 5, "Staging looks healthy ✅")];
        msgs.insert(chan("t1", "ch3"), vec![e1]);
        msgs.insert(chan("t2", "ch4"), vec![msg(&mut seq, &nestor, 60 * 100, "Kickoff notes are posted. Welcome aboard, everyone!")]);
        msgs.insert(chan("t2", "ch5"), vec![]);

        let today = Local::now().date_naive();
        let at = |day: i64, h: u32, m: u32| -> DateTime<Local> {
            (today + Duration::days(day)).and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap()).and_local_timezone(Local).earliest().unwrap_or_else(Local::now)
        };
        let mut ev_seq = 0;
        let mut ev = |subject: &str, start: DateTime<Local>, mins: i64, organizer: &User, online: bool| {
            ev_seq += 1;
            CalEvent {
                id: format!("e{ev_seq}"),
                subject: subject.into(),
                start,
                end: start + Duration::minutes(mins),
                all_day: false,
                location: if online { "Microsoft Teams Meeting".into() } else { "Conf Room Rainier".into() },
                organizer: organizer.name.clone(),
                attendees: vec![organizer.name.clone(), "Alex Wilber".into(), "Megan Bowen".into()],
                join_url: online.then(|| "https://teams.microsoft.com/l/meetup-join/demo".to_string()),
                web_link: "https://outlook.office.com/calendar".into(),
                preview: "Agenda and notes will be shared in the meeting chat.".into(),
                response: "accepted".into(),
                cancelled: false,
            }
        };
        let mut events = vec![
            ev("Daily standup", at(0, 9, 30), 30, &diego, true),
            ev("Design review", at(0, 14, 0), 60, &megan, true),
            ev("1:1 Alex / Adele", at(1, 11, 0), 30, &adele, true),
            ev("Daily standup", at(1, 9, 30), 30, &diego, true),
            ev("Sprint planning", at(2, 10, 0), 90, &patti, true),
            ev("Lunch with Lynne", at(2, 12, 0), 60, &lynne, false),
            ev("Launch retro", at(-1, 15, 0), 60, &patti, true),
            ev("Customer call: Fabrikam", at(3, 16, 0), 45, &nestor, true),
        ];
        let mut offsite = ev("Team offsite", at(4, 0, 0), 24 * 60, &adele, false);
        offsite.all_day = true;
        offsite.location = "Building 25".into();
        events.push(offsite);

        let presence = HashMap::from([
            ("me".to_string(), Availability::Available),
            ("u1".to_string(), Availability::Available),
            ("u2".to_string(), Availability::Busy),
            ("u3".to_string(), Availability::Away),
            ("u4".to_string(), Availability::DoNotDisturb),
            ("u5".to_string(), Availability::Offline),
            ("u6".to_string(), Availability::BeRightBack),
        ]);
        Self { st: Mutex::new(St { me, people, chats, msgs, teams, events, presence, pending: vec![], seq, }) }
    }
}

impl St {
    /// Deliver simulated replies that are due.
    fn tick(&mut self) {
        let now = Instant::now();
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending).into_iter().partition(|p| p.due <= now);
        self.pending = later;
        for p in due {
            if let ConvRef::Chat(id) = &p.conv {
                if let Some(c) = self.chats.iter_mut().find(|c| &c.id == id) {
                    c.preview = p.msg.body.clone();
                    c.preview_from = p.msg.from.clone();
                    c.last_time = Some(p.msg.created);
                    c.unread = true;
                }
            }
            let list = self.msgs.entry(p.conv).or_default();
            match p.thread.and_then(|t| list.iter_mut().find(|m| m.id == t)) {
                Some(root) => root.replies.push(p.msg),
                None => list.push(p.msg),
            }
        }
        self.chats.sort_by(|a, b| b.last_time.cmp(&a.last_time));
    }

    fn find(&mut self, conv: &ConvRef, id: &MsgId) -> Result<&mut Message> {
        let list = self.msgs.get_mut(conv).ok_or_else(|| anyhow!("conversation not found"))?;
        let found = match &id.parent {
            Some(p) => list.iter_mut().find(|m| &m.id == p).and_then(|m| m.replies.iter_mut().find(|r| r.id == id.id)),
            None => list.iter_mut().find(|m| m.id == id.id),
        };
        found.ok_or_else(|| anyhow!("message not found"))
    }
}

#[async_trait]
impl Backend for Demo {
    async fn me(&self) -> Result<User> {
        Ok(self.st.lock().unwrap().me.clone())
    }

    async fn chats(&self) -> Result<Vec<Chat>> {
        let mut st = self.st.lock().unwrap();
        st.tick();
        Ok(st.chats.clone())
    }

    async fn messages(&self, conv: &ConvRef, _cursor: Option<String>) -> Result<Page> {
        let mut st = self.st.lock().unwrap();
        st.tick();
        Ok(Page { msgs: st.msgs.get(conv).cloned().unwrap_or_default(), next: None })
    }

    async fn send(&self, conv: &ConvRef, thread: Option<String>, out: Outgoing) -> Result<()> {
        let mut st = self.st.lock().unwrap();
        let me = st.me.clone();
        let mut seq = st.seq;
        let mut m = msg(&mut seq, &me, 0, &out.text);
        if let Some((who, text)) = &out.quote {
            m.body = format!("▎ {who}\n▎ {}\n{}", text.lines().next().unwrap_or(""), out.text);
        }
        m.subject = out.subject.clone();
        m.mentions = out.mentions.iter().map(|u| u.id.clone()).collect();
        // Someone else answers a few seconds later.
        let responder = match conv {
            ConvRef::Chat(id) => st.chats.iter().find(|c| &c.id == id).and_then(|c| c.members.iter().find(|u| u.id != me.id).cloned()),
            ConvRef::Channel { .. } => st.people.get(seq as usize % st.people.len()).cloned(),
        };
        if let Some(r) = responder {
            let text = REPLIES[seq as usize % REPLIES.len()];
            let mut reply = msg(&mut seq, &r, 0, text);
            reply.created = Utc::now() + Duration::seconds(3);
            let reply_thread = match conv {
                ConvRef::Chat(_) => None,
                ConvRef::Channel { .. } => Some(thread.clone().unwrap_or_else(|| m.id.clone())),
            };
            st.pending.push(Pending { due: Instant::now() + std::time::Duration::from_secs(3), conv: conv.clone(), thread: reply_thread, msg: reply });
        }
        st.seq = seq;
        if let ConvRef::Chat(id) = conv {
            if let Some(c) = st.chats.iter_mut().find(|c| &c.id == id) {
                c.preview = out.text.lines().next().unwrap_or("").to_string();
                c.preview_from = me.clone();
                c.last_time = Some(m.created);
                c.unread = false;
            }
        }
        let list = st.msgs.entry(conv.clone()).or_default();
        match thread.and_then(|t| list.iter_mut().find(|x| x.id == t)) {
            Some(root) => root.replies.push(m),
            None => list.push(m),
        }
        st.tick();
        Ok(())
    }

    async fn edit(&self, conv: &ConvRef, id: &MsgId, text: &str) -> Result<()> {
        let mut st = self.st.lock().unwrap();
        let m = st.find(conv, id)?;
        m.body = text.to_string();
        m.edited = true;
        Ok(())
    }

    async fn delete(&self, conv: &ConvRef, id: &MsgId) -> Result<()> {
        let mut st = self.st.lock().unwrap();
        let m = st.find(conv, id)?;
        m.deleted = true;
        m.body.clear();
        Ok(())
    }

    async fn react(&self, conv: &ConvRef, id: &MsgId, emoji: &str, add: bool) -> Result<()> {
        let mut st = self.st.lock().unwrap();
        let me = st.me.id.clone();
        let m = st.find(conv, id)?;
        m.reactions.retain(|r| !(r.user_id == me && r.emoji == emoji));
        if add {
            m.reactions.push(Reaction { emoji: emoji.into(), user_id: me });
        }
        Ok(())
    }

    async fn mark_read(&self, chat_id: &str) -> Result<()> {
        let mut st = self.st.lock().unwrap();
        if let Some(c) = st.chats.iter_mut().find(|c| c.id == chat_id) {
            c.unread = false;
        }
        Ok(())
    }

    async fn teams(&self) -> Result<Vec<Team>> {
        Ok(self.st.lock().unwrap().teams.clone())
    }

    async fn events(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<CalEvent>> {
        let st = self.st.lock().unwrap();
        let mut out: Vec<CalEvent> = st.events.iter().filter(|e| e.start.with_timezone(&Utc) < to && e.end.with_timezone(&Utc) > from).cloned().collect();
        out.sort_by_key(|e| e.start);
        Ok(out)
    }

    async fn create_event(&self, ev: NewEvent) -> Result<()> {
        let mut st = self.st.lock().unwrap();
        st.seq += 1;
        let (id, organizer) = (format!("new{}", st.seq), st.me.name.clone());
        st.events.push(CalEvent {
            id,
            subject: ev.subject,
            start: ev.start.with_timezone(&Local),
            end: ev.end.with_timezone(&Local),
            all_day: false,
            location: "Microsoft Teams Meeting".into(),
            organizer,
            attendees: ev.attendees,
            join_url: Some("https://teams.microsoft.com/l/meetup-join/demo".into()),
            web_link: "https://outlook.office.com/calendar".into(),
            preview: String::new(),
            response: "organizer".into(),
            cancelled: false,
        });
        Ok(())
    }

    async fn rsvp(&self, event_id: &str, r: Rsvp) -> Result<()> {
        let mut st = self.st.lock().unwrap();
        let e = st.events.iter_mut().find(|e| e.id == event_id).ok_or_else(|| anyhow!("event not found"))?;
        e.response = match r {
            Rsvp::Accept => "accepted",
            Rsvp::Tentative => "tentativelyAccepted",
            Rsvp::Decline => "declined",
        }
        .into();
        Ok(())
    }

    async fn presence(&self, ids: Vec<String>) -> Result<HashMap<String, Availability>> {
        let st = self.st.lock().unwrap();
        Ok(ids.into_iter().filter_map(|id| st.presence.get(&id).map(|a| (id, *a))).collect())
    }

    async fn set_presence(&self, a: Option<Availability>) -> Result<()> {
        self.st.lock().unwrap().presence.insert("me".into(), a.unwrap_or(Availability::Available));
        Ok(())
    }

    async fn set_status_message(&self, _text: &str) -> Result<()> {
        Ok(())
    }

    async fn search_people(&self, q: &str) -> Result<Vec<User>> {
        let q = q.to_lowercase();
        Ok(self.st.lock().unwrap().people.iter().filter(|u| u.name.to_lowercase().contains(&q) || u.email.contains(&q)).cloned().collect())
    }

    async fn create_chat(&self, users: Vec<User>) -> Result<Chat> {
        let mut st = self.st.lock().unwrap();
        let me = st.me.clone();
        if users.len() == 1 {
            if let Some(c) = st.chats.iter().find(|c| c.kind == ChatKind::OneOnOne && c.members.iter().any(|m| m.id == users[0].id)) {
                return Ok(c.clone());
            }
        }
        st.seq += 1;
        let mut members = users.clone();
        members.push(me);
        let chat = Chat {
            id: format!("new{}", st.seq),
            kind: if users.len() == 1 { ChatKind::OneOnOne } else { ChatKind::Group },
            topic: None,
            members,
            preview: String::new(),
            preview_from: User::default(),
            last_time: Some(Utc::now()),
            unread: false,
        };
        st.msgs.insert(ConvRef::Chat(chat.id.clone()), vec![]);
        st.chats.insert(0, chat.clone());
        Ok(chat)
    }

    async fn search_messages(&self, q: &str) -> Result<Vec<SearchHit>> {
        let q = q.to_lowercase();
        let st = self.st.lock().unwrap();
        let mut out = vec![];
        for (conv, list) in &st.msgs {
            for m in list.iter().flat_map(|m| std::iter::once(m).chain(m.replies.iter())) {
                if m.body.to_lowercase().contains(&q) && !m.system {
                    out.push(SearchHit { conv: Some(conv.clone()), from: m.from.name.clone(), summary: m.body.clone(), when: Some(m.created) });
                }
            }
        }
        out.sort_by(|a, b| b.when.cmp(&a.when));
        Ok(out)
    }

    async fn files(&self, folder: Option<String>) -> Result<Vec<DriveItem>> {
        let item = |id: &str, name: &str, folder: bool, size: u64, days: i64, by: &str| DriveItem {
            id: id.into(),
            name: name.into(),
            folder,
            size,
            modified: Some(Utc::now() - Duration::days(days)),
            modified_by: by.into(),
            web_url: format!("https://contoso.example/files/{}", name.replace(' ', "%20")),
        };
        Ok(match folder.as_deref() {
            None => vec![
                item("f1", "Documents", true, 0, 2, "Alex Wilber"),
                item("f2", "Recordings", true, 0, 9, "Alex Wilber"),
                item("d1", "Q4 Roadmap.pptx", false, 4_812_333, 1, "Megan Bowen"),
                item("d2", "Budget 2026.xlsx", false, 88_120, 3, "Nestor Wilke"),
                item("d3", "Meeting notes.docx", false, 23_904, 0, "Alex Wilber"),
            ],
            Some("f1") => vec![
                item("f3", "Specs", true, 0, 12, "Diego Siciliani"),
                item("d4", "Brand guidelines.pdf", false, 2_300_114, 30, "Lynne Robbins"),
                item("d5", "Launch checklist.xlsx", false, 41_222, 1, "Megan Bowen"),
            ],
            Some("f2") => vec![item("d6", "Weekly product sync.mp4", false, 311_000_000, 2, "Patti Fernandez")],
            Some("f3") => vec![item("d7", "Onboarding flow.md", false, 9_120, 5, "Alex Wilber")],
            _ => vec![],
        })
    }

    async fn download(&self, item: &DriveItem, dest: PathBuf) -> Result<()> {
        std::fs::write(dest, format!("Demo content of {}\n", item.name))?;
        Ok(())
    }

    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        // Let a curious user point the demo at a real photo of their own.
        if let Some(path) = std::env::var_os("TEAMSTUI_DEMO_IMAGE") {
            return Ok(std::fs::read(path)?);
        }
        // Otherwise render a photo-like scene so the inline viewer has something worth seeing.
        let img = scene(!url.contains("twilight") && !url.contains("gif"));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).map_err(|e| anyhow!("{e}"))?;
        Ok(buf.into_inner())
    }
}

fn clamp8(v: f32) -> u8 {
    v.clamp(0.0, 255.0) as u8
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn mix(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (lerp(a.0, b.0, t), lerp(a.1, b.1, t), lerp(a.2, b.2, t))
}

fn hash(x: u32, y: u32) -> f32 {
    let mut v = x.wrapping_mul(374_761_393).wrapping_add(y.wrapping_mul(668_265_263));
    v = (v ^ (v >> 13)).wrapping_mul(1_274_126_177);
    ((v ^ (v >> 16)) & 0xffff) as f32 / 65535.0
}

/// A procedurally generated sunset over the sea — all smooth gradients, a glowing
/// sun, layered mountains and a shimmering reflection, to show off inline images.
/// `warm` is the golden-hour palette; otherwise a cooler twilight (used for the GIF).
fn scene(warm: bool) -> image::RgbImage {
    use image::{Rgb, RgbImage};
    let (w, h) = (1280u32, 720u32);
    let (wf, hf) = (w as f32, h as f32);
    let hy = 0.64 * hf; // horizon
    let sx = 0.54 * wf;
    let sy = 0.84 * hy; // sun sits just above the horizon
    let sunr = 0.085 * hf;

    let (sky_top, sky_mid, sky_hor, sun_col) = if warm {
        ((28.0, 32.0, 78.0), (196.0, 92.0, 70.0), (255.0, 188.0, 96.0), (255.0, 243.0, 205.0))
    } else {
        ((16.0, 20.0, 54.0), (70.0, 60.0, 120.0), (210.0, 140.0, 150.0), (235.0, 220.0, 255.0))
    };
    let sea_near = if warm { (205.0, 120.0, 78.0) } else { (120.0, 90.0, 140.0) };
    let sea_far = if warm { (16.0, 26.0, 62.0) } else { (10.0, 14.0, 44.0) };

    let mut img = RgbImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let (xf, yf) = (x as f32, y as f32);
        let (mut r, mut g, mut b);
        if yf < hy {
            let t = (yf / hy).clamp(0.0, 1.0);
            let c = if t < 0.6 { mix(sky_top, sky_mid, t / 0.6) } else { mix(sky_mid, sky_hor, (t - 0.6) / 0.4) };
            (r, g, b) = c;
            // faint stars high in the sky
            if t < 0.32 && hash(x, y) > 0.9975 {
                let s = 210.0 + 45.0 * hash(x + 9, y + 4);
                r = r.max(s);
                g = g.max(s);
                b = b.max(s);
            }
            // sun disc + soft glow
            let d = ((xf - sx).powi(2) + (yf - sy).powi(2)).sqrt();
            if d < sunr {
                (r, g, b) = sun_col;
            } else {
                let glow = (-(d - sunr) / (1.05 * sunr)).exp();
                r = (r + glow * 215.0).min(255.0);
                g = (g + glow * 165.0).min(255.0);
                b = (b + glow * (if warm { 90.0 } else { 150.0 })).min(255.0);
            }
            // two mountain ridges fading into haze near the horizon
            let ridge_far = hy - 0.165 * hf - 0.05 * hf * (xf * 0.0105).sin() - 0.022 * hf * (xf * 0.031 + 1.3).sin();
            let ridge_near = hy - 0.075 * hf - 0.04 * hf * (xf * 0.008 + 2.1).sin() - 0.018 * hf * (xf * 0.025).sin();
            if yf > ridge_near {
                (r, g, b) = (38.0, 30.0, 58.0);
            } else if yf > ridge_far {
                let haze = 0.55;
                (r, g, b) = mix((r, g, b), (74.0, 58.0, 100.0), haze);
            }
        } else {
            let t = ((yf - hy) / (hf - hy)).clamp(0.0, 1.0);
            (r, g, b) = mix(sea_near, sea_far, t.powf(0.8));
            // gentle horizontal wave banding
            let band = ((yf * 0.22).sin() * 0.5 + 0.5) * 9.0;
            r += band;
            g += band * 0.6;
            // the sun's reflection: a widening, flickering column of light
            let dx = (xf - sx).abs();
            let width = sunr * (0.55 + 2.4 * t);
            if dx < width {
                let flick = 0.45 + 0.55 * ((yf * 0.7).sin() * hash(x, (y / 3) * 3));
                let m = (1.0 - dx / width) * flick;
                r = (r + m * 180.0).min(255.0);
                g = (g + m * 135.0).min(255.0);
                b = (b + m * (if warm { 70.0 } else { 120.0 })).min(255.0);
            }
        }
        *p = Rgb([clamp8(r), clamp8(g), clamp8(b)]);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn dump_scenes() {
        for (warm, name) in [(true, "sunset"), (false, "twilight")] {
            let img = scene(warm);
            let mut buf = std::io::Cursor::new(Vec::new());
            img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
            std::fs::write(format!("/tmp/claude-0/-home-user-TeamsTUI/c17f41bd-c061-5cf2-9c2b-c8be21dc23f4/scratchpad/demo_{name}.png"), buf.into_inner()).unwrap();
        }
    }

    #[test]
    fn scene_is_a_rich_image() {
        let img = scene(true);
        assert_eq!(img.dimensions(), (1280, 720));
        let colors: HashSet<(u8, u8, u8)> = img.pixels().map(|p| (p[0], p[1], p[2])).collect();
        // A real gradient scene has thousands of distinct colors, not a flat fill.
        assert!(colors.len() > 3000, "scene looks flat: only {} colors", colors.len());
    }
}
