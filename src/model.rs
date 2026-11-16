//! Plain data types shared by the backends and the UI.
use chrono::{DateTime, Local, Utc};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub email: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Availability {
    Available,
    Busy,
    DoNotDisturb,
    BeRightBack,
    Away,
    Offline,
    #[default]
    Unknown,
}

impl Availability {
    pub const SETTABLE: [Availability; 6] = [
        Availability::Available,
        Availability::Busy,
        Availability::DoNotDisturb,
        Availability::BeRightBack,
        Availability::Away,
        Availability::Offline,
    ];

    pub fn parse(s: &str) -> Self {
        match s {
            "Available" | "AvailableIdle" => Self::Available,
            "Busy" | "BusyIdle" => Self::Busy,
            "DoNotDisturb" => Self::DoNotDisturb,
            "BeRightBack" => Self::BeRightBack,
            "Away" => Self::Away,
            "Offline" => Self::Offline,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::Busy => "Busy",
            Self::DoNotDisturb => "Do not disturb",
            Self::BeRightBack => "Be right back",
            Self::Away => "Appear away",
            Self::Offline => "Appear offline",
            Self::Unknown => "Status unknown",
        }
    }

    /// How someone else's presence reads ("Away" rather than "Appear away").
    pub fn status(self) -> &'static str {
        match self {
            Self::Away => "Away",
            Self::Offline => "Offline",
            other => other.label(),
        }
    }

    /// (availability, activity) pair accepted by setUserPreferredPresence.
    pub fn graph(self) -> (&'static str, &'static str) {
        match self {
            Self::Available | Self::Unknown => ("Available", "Available"),
            Self::Busy => ("Busy", "Busy"),
            Self::DoNotDisturb => ("DoNotDisturb", "DoNotDisturb"),
            Self::BeRightBack => ("BeRightBack", "BeRightBack"),
            Self::Away => ("Away", "Away"),
            Self::Offline => ("Offline", "OffWork"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatKind {
    OneOnOne,
    Group,
    Meeting,
}

#[derive(Clone, Debug)]
pub struct Chat {
    pub id: String,
    pub kind: ChatKind,
    pub topic: Option<String>,
    pub members: Vec<User>,
    pub preview: String,
    pub preview_from: User,
    pub last_time: Option<DateTime<Utc>>,
    pub unread: bool,
}

impl Chat {
    pub fn others<'a>(&'a self, me: &'a str) -> impl Iterator<Item = &'a User> {
        self.members.iter().filter(move |m| m.id != me)
    }

    pub fn title(&self, me: &str) -> String {
        if let Some(t) = self.topic.as_ref().filter(|t| !t.is_empty()) {
            return t.clone();
        }
        let names: Vec<&str> = self.others(me).map(|m| m.name.as_str()).filter(|n| !n.is_empty()).collect();
        if names.is_empty() {
            match self.kind {
                ChatKind::Meeting => "Meeting chat".into(),
                _ => "Chat".into(),
            }
        } else {
            names.join(", ")
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reaction {
    pub emoji: String,
    pub user_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub name: String,
    pub url: String,
}

impl Attachment {
    /// True when the attachment points at an image or GIF we can display.
    pub fn is_image(&self) -> bool {
        let u = self.url.split(['?', '#']).next().unwrap_or(&self.url).to_ascii_lowercase();
        [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"].iter().any(|e| u.ends_with(e))
            || self.url.contains("hostedContents")
            || self.url.contains("giphy")
    }

    pub fn is_gif(&self) -> bool {
        self.url.to_ascii_lowercase().contains(".gif") || self.url.contains("giphy") || self.name.eq_ignore_ascii_case("gif")
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub id: String,
    pub from: User,
    pub subject: Option<String>,
    pub body: String,
    pub created: DateTime<Utc>,
    pub edited: bool,
    pub deleted: bool,
    pub system: bool,
    pub reactions: Vec<Reaction>,
    pub attachments: Vec<Attachment>,
    pub mentions: Vec<String>,
    pub replies: Vec<Message>,
}

/// Identifies a message; `parent` is set for replies inside a channel thread.
#[derive(Clone, Debug, PartialEq)]
pub struct MsgId {
    pub id: String,
    pub parent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConvRef {
    Chat(String),
    Channel { team: String, channel: String },
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub msgs: Vec<Message>,
    /// Opaque cursor for loading older messages.
    pub next: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Outgoing {
    pub text: String,
    pub mentions: Vec<User>,
    /// (author, text) of a quoted message.
    pub quote: Option<(String, String)>,
    pub subject: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug)]
pub struct CalEvent {
    pub id: String,
    pub subject: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub all_day: bool,
    pub location: String,
    pub organizer: String,
    pub attendees: Vec<String>,
    pub join_url: Option<String>,
    pub web_link: String,
    pub preview: String,
    pub response: String,
    pub cancelled: bool,
}

#[derive(Clone, Debug)]
pub struct NewEvent {
    pub subject: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub attendees: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rsvp {
    Accept,
    Tentative,
    Decline,
}

#[derive(Clone, Debug)]
pub struct DriveItem {
    pub id: String,
    pub name: String,
    pub folder: bool,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    pub modified_by: String,
    pub web_url: String,
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub conv: Option<ConvRef>,
    pub from: String,
    pub summary: String,
    pub when: Option<DateTime<Utc>>,
}
