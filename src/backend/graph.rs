//! Microsoft Graph implementation of the backend.
use super::auth::Auth;
use super::Backend;
use crate::html;
use crate::model::*;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local, NaiveDateTime, Utc};
use reqwest::Method;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

const BASE: &str = "https://graph.microsoft.com/v1.0";

pub struct Graph {
    auth: Arc<Auth>,
    http: reqwest::Client,
    me: OnceCell<User>,
}

/// Percent-encode a query-string value.
pub fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn st(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn dt(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|d| d.with_timezone(&Utc))
}

fn items(v: &Value) -> impl Iterator<Item = &Value> {
    v["value"].as_array().into_iter().flatten()
}

fn user(v: &Value) -> User {
    User {
        id: st(&v["id"]),
        name: st(&v["displayName"]),
        email: v["mail"].as_str().or(v["userPrincipalName"].as_str()).unwrap_or_default().to_string(),
    }
}

fn body_text(body: &Value) -> String {
    let content = body["content"].as_str().unwrap_or_default();
    if body["contentType"].as_str().unwrap_or("text").eq_ignore_ascii_case("html") {
        html::to_text(content)
    } else {
        content.to_string()
    }
}

fn reaction_emoji(kind: &str) -> String {
    match kind {
        "like" => "👍",
        "heart" => "❤️",
        "laugh" => "😆",
        "surprised" => "😮",
        "sad" => "😢",
        "angry" => "😠",
        other => other,
    }
    .to_string()
}

/// "#microsoft.graph.membersAddedEventMessageDetail" -> "Members added"
fn humanize_event(odata_type: &str) -> String {
    let name = odata_type.rsplit('.').next().unwrap_or(odata_type).trim_end_matches("EventMessageDetail");
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
        }
        if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
    }
    if out.is_empty() {
        "System event".into()
    } else {
        out
    }
}

pub fn parse_message(v: &Value) -> Message {
    let from = if v["from"]["user"].is_object() {
        User { id: st(&v["from"]["user"]["id"]), name: st(&v["from"]["user"]["displayName"]), email: String::new() }
    } else {
        let app = &v["from"]["application"];
        User { id: st(&app["id"]), name: app["displayName"].as_str().unwrap_or("").to_string(), email: String::new() }
    };
    let system = v["messageType"].as_str().is_some_and(|t| t != "message");
    let mut body = if system && v["eventDetail"].is_object() {
        humanize_event(v["eventDetail"]["@odata.type"].as_str().unwrap_or(""))
    } else {
        body_text(&v["body"])
    };
    let mut attachments = vec![];
    for a in v["attachments"].as_array().into_iter().flatten() {
        let ctype = a["contentType"].as_str().unwrap_or("");
        if ctype == "messageReference" {
            // A quoted reply: surface the quoted preview above the body.
            if let Ok(q) = serde_json::from_str::<Value>(a["content"].as_str().unwrap_or("")) {
                let who = q["messageSender"]["user"]["displayName"].as_str().unwrap_or("");
                let preview = q["messagePreview"].as_str().unwrap_or("");
                body = format!("▎ {who}\n▎ {preview}\n{body}");
            }
        } else if ctype == "reference" || a["contentUrl"].is_string() {
            attachments.push(Attachment { name: a["name"].as_str().unwrap_or("Attachment").to_string(), url: st(&a["contentUrl"]) });
        } else if ctype.contains("card") {
            let text = serde_json::from_str::<Value>(a["content"].as_str().unwrap_or(""))
                .ok()
                .and_then(|c| c["title"].as_str().or(c["text"].as_str()).map(html::to_text))
                .unwrap_or_default();
            attachments.push(Attachment { name: if text.is_empty() { "Card".into() } else { format!("Card: {text}") }, url: String::new() });
        }
    }
    if !system {
        for (src, gif) in html::images(v["body"]["content"].as_str().unwrap_or("")) {
            attachments.push(Attachment { name: if gif { "GIF".into() } else { "Image".into() }, url: src });
        }
    }
    let mut replies: Vec<Message> = v["replies"].as_array().into_iter().flatten().map(parse_message).collect();
    replies.sort_by_key(|m| m.created);
    Message {
        id: st(&v["id"]),
        from,
        subject: v["subject"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
        body,
        created: dt(&v["createdDateTime"]).unwrap_or_default(),
        edited: v["lastEditedDateTime"].is_string(),
        deleted: v["deletedDateTime"].is_string(),
        system,
        reactions: v["reactions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| Reaction { emoji: reaction_emoji(r["reactionType"].as_str().unwrap_or("")), user_id: st(&r["user"]["user"]["id"]) })
            .collect(),
        attachments,
        mentions: v["mentions"].as_array().into_iter().flatten().map(|m| st(&m["mentioned"]["user"]["id"])).collect(),
        replies,
    }
}

pub fn parse_chat(v: &Value, me: &str) -> Chat {
    let p = &v["lastMessagePreview"];
    let from = User { id: st(&p["from"]["user"]["id"]), name: st(&p["from"]["user"]["displayName"]), email: String::new() };
    let last_time = dt(&p["createdDateTime"]).or_else(|| dt(&v["lastUpdatedDateTime"]));
    let preview = if p["isDeleted"] == true {
        "This message has been deleted".to_string()
    } else {
        body_text(&p["body"]).lines().next().unwrap_or("").to_string()
    };
    let unread = p.is_object()
        && from.id != me
        && match (dt(&p["createdDateTime"]), dt(&v["viewpoint"]["lastMessageReadDateTime"])) {
            (Some(l), Some(r)) => l > r,
            _ => false,
        };
    Chat {
        id: st(&v["id"]),
        kind: match v["chatType"].as_str().unwrap_or("") {
            "oneOnOne" => ChatKind::OneOnOne,
            "meeting" => ChatKind::Meeting,
            _ => ChatKind::Group,
        },
        topic: v["topic"].as_str().map(str::to_string),
        members: v["members"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| User { id: st(&m["userId"]), name: st(&m["displayName"]), email: st(&m["email"]) })
            .collect(),
        preview,
        preview_from: from,
        last_time,
        unread,
    }
}

fn parse_event(v: &Value) -> Option<CalEvent> {
    let when = |x: &Value| -> Option<DateTime<Local>> {
        let n = NaiveDateTime::parse_from_str(x["dateTime"].as_str()?, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
        Some(n.and_utc().with_timezone(&Local))
    };
    let all_day = v["isAllDay"] == true;
    let (mut start, mut end) = (when(&v["start"])?, when(&v["end"])?);
    if all_day {
        // All-day events are midnight-to-midnight in the calendar's zone; keep the dates.
        let fix = |x: &Value| -> Option<DateTime<Local>> {
            let n = NaiveDateTime::parse_from_str(x["dateTime"].as_str()?, "%Y-%m-%dT%H:%M:%S%.f").ok()?;
            n.and_local_timezone(Local).earliest()
        };
        start = fix(&v["start"]).unwrap_or(start);
        end = fix(&v["end"]).unwrap_or(end);
    }
    Some(CalEvent {
        id: st(&v["id"]),
        subject: v["subject"].as_str().filter(|s| !s.is_empty()).unwrap_or("(No title)").to_string(),
        start,
        end,
        all_day,
        location: st(&v["location"]["displayName"]),
        organizer: st(&v["organizer"]["emailAddress"]["name"]),
        attendees: v["attendees"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| {
                let e = &a["emailAddress"];
                e["name"].as_str().filter(|n| !n.is_empty()).or(e["address"].as_str()).unwrap_or("").to_string()
            })
            .collect(),
        join_url: v["onlineMeeting"]["joinUrl"].as_str().map(str::to_string),
        web_link: st(&v["webLink"]),
        preview: st(&v["bodyPreview"]),
        response: st(&v["responseStatus"]["response"]),
        cancelled: v["isCancelled"] == true,
    })
}

fn msg_base(conv: &ConvRef) -> String {
    match conv {
        ConvRef::Chat(id) => format!("/chats/{id}/messages"),
        ConvRef::Channel { team, channel } => format!("/teams/{team}/channels/{channel}/messages"),
    }
}

fn msg_path(conv: &ConvRef, id: &MsgId) -> String {
    match &id.parent {
        Some(p) => format!("{}/{p}/replies/{}", msg_base(conv), id.id),
        None => format!("{}/{}", msg_base(conv), id.id),
    }
}

/// Build the HTML body (and mentions array) for an outgoing message.
pub fn outgoing_body(out: &Outgoing) -> Value {
    let mut content = html::escape(&out.text);
    let mut mentions = vec![];
    for u in &out.mentions {
        let needle = format!("@{}", html::escape(&u.name));
        if content.contains(&needle) {
            let idx = mentions.len();
            content = content.replace(&needle, &format!("<at id=\"{idx}\">{}</at>", html::escape(&u.name)));
            mentions.push(json!({
                "id": idx,
                "mentionText": u.name,
                "mentioned": { "user": { "id": u.id, "displayName": u.name, "userIdentityType": "aadUser" } }
            }));
        }
    }
    if let Some((who, text)) = &out.quote {
        content = format!("<blockquote><b>{}</b><br>{}</blockquote>{content}", html::escape(who), html::escape(text));
    }
    let mut v = json!({ "body": { "contentType": "html", "content": content } });
    if !mentions.is_empty() {
        v["mentions"] = Value::Array(mentions);
    }
    if let Some(s) = out.subject.as_ref().filter(|s| !s.is_empty()) {
        v["subject"] = json!(s);
    }
    v
}

impl Graph {
    pub fn new(auth: Arc<Auth>) -> Self {
        Self { auth, http: reqwest::Client::new(), me: OnceCell::new() }
    }

    async fn call(&self, method: Method, url: &str, body: Option<&Value>, headers: &[(&str, &str)]) -> Result<Value> {
        let resp = self.raw(method, url, body, headers).await?;
        let text = resp.text().await.unwrap_or_default();
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn raw(&self, method: Method, url: &str, body: Option<&Value>, headers: &[(&str, &str)]) -> Result<reqwest::Response> {
        let url = if url.starts_with("http") { url.to_string() } else { format!("{BASE}{url}") };
        let mut force = false;
        for attempt in 0..4 {
            let token = self.auth.access_token(force).await?;
            let mut rq = self.http.request(method.clone(), &url).bearer_auth(token).timeout(Duration::from_secs(60));
            for (k, v) in headers {
                rq = rq.header(*k, *v);
            }
            if let Some(b) = body {
                rq = rq.json(b);
            }
            let resp = rq.send().await.map_err(|e| anyhow!("network error: {e}"))?;
            let status = resp.status().as_u16();
            if status == 401 && !force {
                force = true;
                continue;
            }
            if matches!(status, 429 | 503 | 504) && attempt < 3 {
                let wait = resp.headers().get("retry-after").and_then(|h| h.to_str().ok()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(2);
                tokio::time::sleep(Duration::from_secs(wait.clamp(1, 15))).await;
                continue;
            }
            if resp.status().is_success() {
                return Ok(resp);
            }
            let text = resp.text().await.unwrap_or_default();
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let code = v["error"]["code"].as_str().unwrap_or("");
            let msg = v["error"]["message"].as_str().unwrap_or(text.as_str());
            let hint = if status == 403 { " (missing permission or admin consent?)" } else { "" };
            bail!("{status} {code}: {msg}{hint}");
        }
        bail!("Microsoft Graph kept throttling the request")
    }

    async fn get(&self, url: &str) -> Result<Value> {
        self.call(Method::GET, url, None, &[]).await
    }

    async fn post(&self, url: &str, body: Value) -> Result<Value> {
        self.call(Method::POST, url, Some(&body), &[]).await
    }

    async fn my_id(&self) -> Result<String> {
        Ok(self.me().await?.id)
    }
}

#[async_trait]
impl Backend for Graph {
    async fn me(&self) -> Result<User> {
        self.me.get_or_try_init(|| async { Ok::<_, anyhow::Error>(user(&self.get("/me?$select=id,displayName,mail,userPrincipalName").await?)) }).await.cloned()
    }

    async fn chats(&self) -> Result<Vec<Chat>> {
        let me = self.my_id().await?;
        let order = enc("lastMessagePreview/createdDateTime desc");
        // Some tenants reject the combined expand; degrade gracefully.
        let attempts = [
            format!("/me/chats?$top=50&$expand={}&$orderby={order}", enc("members,lastMessagePreview")),
            format!("/me/chats?$top=50&$expand=lastMessagePreview&$orderby={order}"),
            "/me/chats?$top=50&$expand=members".to_string(),
        ];
        let mut last_err = None;
        for (i, url) in attempts.iter().enumerate() {
            match self.get(url).await {
                Ok(v) => {
                    let mut chats: Vec<Chat> = items(&v).map(|c| parse_chat(c, &me)).collect();
                    if i == 1 {
                        // Members were not expanded: fetch them per chat.
                        let lists = futures::future::join_all(chats.iter().map(|c| self.get_members(&c.id))).await;
                        for (c, m) in chats.iter_mut().zip(lists) {
                            c.members = m.unwrap_or_default();
                        }
                    }
                    chats.sort_by(|a, b| b.last_time.cmp(&a.last_time));
                    return Ok(chats);
                }
                Err(e) if e.to_string().starts_with("400") => last_err = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("could not list chats")))
    }

    async fn messages(&self, conv: &ConvRef, cursor: Option<String>) -> Result<Page> {
        let v = match (&cursor, conv) {
            (Some(next), _) => self.get(next).await?,
            (None, ConvRef::Chat(_)) => self.get(&format!("{}?$top=50", msg_base(conv))).await?,
            (None, ConvRef::Channel { .. }) => {
                let base = msg_base(conv);
                match self.get(&format!("{base}?$top=30&$expand=replies")).await {
                    Ok(v) => v,
                    Err(e) if e.to_string().starts_with("400") => self.get(&format!("{base}?$top=30")).await?,
                    Err(e) => return Err(e),
                }
            }
        };
        Ok(Page { msgs: items(&v).map(parse_message).collect(), next: v["@odata.nextLink"].as_str().map(str::to_string) })
    }

    async fn send(&self, conv: &ConvRef, thread: Option<String>, out: Outgoing) -> Result<()> {
        let url = match thread {
            Some(root) => format!("{}/{root}/replies", msg_base(conv)),
            None => msg_base(conv),
        };
        self.post(&url, outgoing_body(&out)).await.map(|_| ())
    }

    async fn edit(&self, conv: &ConvRef, id: &MsgId, text: &str) -> Result<()> {
        let body = json!({ "body": { "contentType": "html", "content": html::escape(text) } });
        self.call(Method::PATCH, &msg_path(conv, id), Some(&body), &[]).await.map(|_| ())
    }

    async fn delete(&self, conv: &ConvRef, id: &MsgId) -> Result<()> {
        let url = match conv {
            ConvRef::Chat(chat) => format!("/users/{}/chats/{chat}/messages/{}/softDelete", self.my_id().await?, id.id),
            ConvRef::Channel { .. } => format!("{}/softDelete", msg_path(conv, id)),
        };
        self.call(Method::POST, &url, None, &[]).await.map(|_| ())
    }

    async fn react(&self, conv: &ConvRef, id: &MsgId, emoji: &str, add: bool) -> Result<()> {
        let action = if add { "setReaction" } else { "unsetReaction" };
        self.post(&format!("{}/{action}", msg_path(conv, id)), json!({ "reactionType": emoji })).await.map(|_| ())
    }

    async fn mark_read(&self, chat_id: &str) -> Result<()> {
        let me = self.my_id().await?;
        let tenant = self.auth.tenant_id().await.ok_or_else(|| anyhow!("unknown tenant"))?;
        self.post(&format!("/chats/{chat_id}/markChatReadForUser"), json!({ "user": { "id": me, "tenantId": tenant } })).await.map(|_| ())
    }

    async fn teams(&self) -> Result<Vec<Team>> {
        let v = self.get("/me/joinedTeams?$select=id,displayName").await?;
        let mut teams: Vec<Team> = items(&v).map(|t| Team { id: st(&t["id"]), name: st(&t["displayName"]), channels: vec![] }).collect();
        let lists = futures::future::join_all(teams.iter().map(|t| self.get_owned(format!("/teams/{}/channels?$select=id,displayName,description", t.id)))).await;
        for (t, l) in teams.iter_mut().zip(lists) {
            if let Ok(v) = l {
                t.channels = items(&v).map(|c| Channel { id: st(&c["id"]), name: st(&c["displayName"]), description: st(&c["description"]) }).collect();
                // "General" first, like Teams.
                t.channels.sort_by_key(|c| (c.name != "General", c.name.to_lowercase()));
            }
        }
        teams.sort_by_key(|t| t.name.to_lowercase());
        Ok(teams)
    }

    async fn events(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<CalEvent>> {
        let f = "%Y-%m-%dT%H:%M:%SZ";
        let url = format!(
            "/me/calendarView?startDateTime={}&endDateTime={}&$top=200&$orderby={}",
            enc(&from.format(f).to_string()),
            enc(&to.format(f).to_string()),
            enc("start/dateTime")
        );
        let v = self.call(Method::GET, &url, None, &[("Prefer", "outlook.timezone=\"UTC\"")]).await?;
        Ok(items(&v).filter_map(parse_event).collect())
    }

    async fn create_event(&self, ev: NewEvent) -> Result<()> {
        let f = "%Y-%m-%dT%H:%M:%S";
        let body = json!({
            "subject": ev.subject,
            "start": { "dateTime": ev.start.format(f).to_string(), "timeZone": "UTC" },
            "end": { "dateTime": ev.end.format(f).to_string(), "timeZone": "UTC" },
            "attendees": ev.attendees.iter().map(|a| json!({ "emailAddress": { "address": a }, "type": "required" })).collect::<Vec<_>>(),
            "isOnlineMeeting": true,
            "onlineMeetingProvider": "teamsForBusiness",
        });
        self.post("/me/events", body).await.map(|_| ())
    }

    async fn rsvp(&self, event_id: &str, r: Rsvp) -> Result<()> {
        let action = match r {
            Rsvp::Accept => "accept",
            Rsvp::Tentative => "tentativelyAccept",
            Rsvp::Decline => "decline",
        };
        self.post(&format!("/me/events/{event_id}/{action}"), json!({ "sendResponse": true })).await.map(|_| ())
    }

    async fn presence(&self, ids: Vec<String>) -> Result<HashMap<String, Availability>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let v = self.post("/communications/getPresencesByUserId", json!({ "ids": ids })).await?;
        Ok(items(&v).map(|p| (st(&p["id"]), Availability::parse(p["availability"].as_str().unwrap_or("")))).collect())
    }

    async fn set_presence(&self, a: Option<Availability>) -> Result<()> {
        let me = self.my_id().await?;
        match a {
            Some(a) => {
                let (availability, activity) = a.graph();
                self.post(&format!("/users/{me}/presence/setUserPreferredPresence"), json!({ "availability": availability, "activity": activity })).await?;
            }
            None => {
                self.post(&format!("/users/{me}/presence/clearUserPreferredPresence"), json!({})).await?;
            }
        }
        Ok(())
    }

    async fn set_status_message(&self, text: &str) -> Result<()> {
        let me = self.my_id().await?;
        let body = json!({ "statusMessage": { "message": { "content": text, "contentType": "text" } } });
        self.post(&format!("/users/{me}/presence/setStatusMessage"), body).await.map(|_| ())
    }

    async fn search_people(&self, q: &str) -> Result<Vec<User>> {
        let q = q.replace(['"', '\\'], " ");
        let search = format!("\"displayName:{q}\" OR \"mail:{q}\"");
        let url = format!("/users?$search={}&$top=10&$select=id,displayName,mail,userPrincipalName", enc(&search));
        let v = self.call(Method::GET, &url, None, &[("ConsistencyLevel", "eventual")]).await?;
        let me = self.my_id().await?;
        Ok(items(&v).map(user).filter(|u| u.id != me).collect())
    }

    async fn create_chat(&self, users: Vec<User>) -> Result<Chat> {
        let me = self.my_id().await?;
        let member = |id: &str| {
            json!({
                "@odata.type": "#microsoft.graph.aadUserConversationMember",
                "roles": ["owner"],
                "user@odata.bind": format!("{BASE}/users('{id}')"),
            })
        };
        let mut members = vec![member(&me)];
        members.extend(users.iter().map(|u| member(&u.id)));
        let kind = if users.len() == 1 { "oneOnOne" } else { "group" };
        let v = self.post("/chats", json!({ "chatType": kind, "members": members })).await?;
        let mut chat = parse_chat(&v, &me);
        chat.members = users;
        chat.members.push(self.me().await?);
        Ok(chat)
    }

    async fn search_messages(&self, q: &str) -> Result<Vec<SearchHit>> {
        let body = json!({ "requests": [{ "entityTypes": ["chatMessage"], "query": { "queryString": q }, "from": 0, "size": 25 }] });
        let v = self.post("/search/query", body).await?;
        let mut out = vec![];
        for container in v["value"][0]["hitsContainers"].as_array().into_iter().flatten() {
            for h in container["hits"].as_array().into_iter().flatten() {
                let r = &h["resource"];
                let conv = if let Some(chat) = r["chatId"].as_str() {
                    Some(ConvRef::Chat(chat.to_string()))
                } else if r["channelIdentity"]["channelId"].is_string() {
                    Some(ConvRef::Channel { team: st(&r["channelIdentity"]["teamId"]), channel: st(&r["channelIdentity"]["channelId"]) })
                } else {
                    None
                };
                let summary = html::to_text(h["summary"].as_str().unwrap_or(""));
                out.push(SearchHit {
                    conv,
                    from: r["from"]["emailAddress"]["name"].as_str().or(r["from"]["user"]["displayName"].as_str()).unwrap_or("").to_string(),
                    summary: if summary.is_empty() { body_text(&r["body"]) } else { summary },
                    when: dt(&r["createdDateTime"]),
                });
            }
        }
        Ok(out)
    }

    async fn files(&self, folder: Option<String>) -> Result<Vec<DriveItem>> {
        let url = match folder {
            Some(id) => format!("/me/drive/items/{id}/children?$top=200"),
            None => "/me/drive/root/children?$top=200".to_string(),
        };
        let v = self.get(&url).await?;
        let mut out: Vec<DriveItem> = items(&v)
            .map(|i| DriveItem {
                id: st(&i["id"]),
                name: st(&i["name"]),
                folder: i["folder"].is_object(),
                size: i["size"].as_u64().unwrap_or(0),
                modified: dt(&i["lastModifiedDateTime"]),
                modified_by: st(&i["lastModifiedBy"]["user"]["displayName"]),
                web_url: st(&i["webUrl"]),
            })
            .collect();
        out.sort_by_key(|i| (!i.folder, i.name.to_lowercase()));
        Ok(out)
    }

    async fn download(&self, item: &DriveItem, dest: PathBuf) -> Result<()> {
        let resp = self.raw(Method::GET, &format!("/me/drive/items/{}/content", item.id), None, &[]).await?;
        let bytes = resp.bytes().await?;
        tokio::task::spawn_blocking(move || std::fs::write(dest, bytes)).await??;
        Ok(())
    }

    async fn fetch(&self, url: &str, dest: PathBuf) -> Result<()> {
        let resp = if url.starts_with("https://graph.microsoft.com") {
            self.raw(Method::GET, url, None, &[]).await?
        } else {
            self.http.get(url).timeout(Duration::from_secs(60)).send().await.map_err(|e| anyhow!("network error: {e}"))?.error_for_status()?
        };
        let bytes = resp.bytes().await?;
        tokio::task::spawn_blocking(move || std::fs::write(dest, bytes)).await??;
        Ok(())
    }
}

impl Graph {
    async fn get_owned(&self, url: String) -> Result<Value> {
        self.get(&url).await
    }

    async fn get_members(&self, chat_id: &str) -> Result<Vec<User>> {
        let v = self.get(&format!("/chats/{chat_id}/members")).await?;
        Ok(items(&v).map(|m| User { id: st(&m["userId"]), name: st(&m["displayName"]), email: st(&m["email"]) }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chat_message() {
        let v = json!({
            "id": "1700", "messageType": "message", "createdDateTime": "2026-11-10T09:15:00.12Z",
            "lastEditedDateTime": "2026-11-10T09:16:00Z", "deletedDateTime": null,
            "from": { "user": { "id": "u1", "displayName": "Megan Bowen" } },
            "body": { "contentType": "html", "content": "<p>Hi <at id=\"0\">Alex</at> &amp; all</p>" },
            "mentions": [{ "id": 0, "mentioned": { "user": { "id": "me" } } }],
            "reactions": [{ "reactionType": "like", "user": { "user": { "id": "u2" } } }],
            "attachments": [{ "contentType": "reference", "name": "Plan.docx", "contentUrl": "https://x/Plan.docx" }]
        });
        let m = parse_message(&v);
        assert_eq!(m.body, "Hi @Alex & all");
        assert!(m.edited && !m.deleted && !m.system);
        assert_eq!(m.mentions, vec!["me"]);
        assert_eq!(m.reactions[0].emoji, "👍");
        assert_eq!(m.attachments[0].name, "Plan.docx");
    }

    #[test]
    fn parses_system_message_and_chat() {
        let v = json!({ "id": "1", "messageType": "systemEventMessage", "createdDateTime": "2026-11-10T09:15:00Z",
            "body": { "contentType": "html", "content": "<systemEventMessage/>" },
            "eventDetail": { "@odata.type": "#microsoft.graph.membersAddedEventMessageDetail" } });
        let m = parse_message(&v);
        assert!(m.system);
        assert_eq!(m.body, "Members added");

        let c = json!({ "id": "19:x", "chatType": "oneOnOne", "topic": null,
            "viewpoint": { "lastMessageReadDateTime": "2026-11-10T09:00:00Z" },
            "members": [{ "userId": "me", "displayName": "Me" }, { "userId": "u1", "displayName": "Megan Bowen", "email": "m@x.io" }],
            "lastMessagePreview": { "createdDateTime": "2026-11-10T09:15:00Z", "isDeleted": false,
                "body": { "contentType": "text", "content": "ping" }, "from": { "user": { "id": "u1", "displayName": "Megan Bowen" } } } });
        let chat = parse_chat(&c, "me");
        assert!(chat.unread);
        assert_eq!(chat.title("me"), "Megan Bowen");
        assert_eq!(chat.preview, "ping");
    }

    #[test]
    fn builds_outgoing_body_with_mention_and_quote() {
        let out = Outgoing {
            text: "hey @Megan Bowen <3".into(),
            mentions: vec![User { id: "u1".into(), name: "Megan Bowen".into(), email: String::new() }],
            quote: Some(("Megan Bowen".into(), "ping".into())),
            subject: None,
        };
        let v = outgoing_body(&out);
        assert_eq!(v["body"]["content"], "<blockquote><b>Megan Bowen</b><br>ping</blockquote>hey <at id=\"0\">Megan Bowen</at> &lt;3");
        assert_eq!(v["mentions"][0]["mentioned"]["user"]["id"], "u1");
    }

    #[test]
    fn parses_calendar_event() {
        let v = json!({ "id": "e", "subject": "Sync", "isAllDay": false,
            "start": { "dateTime": "2026-11-16T09:00:00.0000000", "timeZone": "UTC" },
            "end": { "dateTime": "2026-11-16T09:30:00.0000000", "timeZone": "UTC" },
            "onlineMeeting": { "joinUrl": "https://teams.microsoft.com/l/meetup-join/x" } });
        let e = parse_event(&v).unwrap();
        assert_eq!((e.end - e.start).num_minutes(), 30);
        assert!(e.join_url.is_some());
    }
}
