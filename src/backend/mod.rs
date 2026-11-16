//! Backend abstraction: the real Microsoft Graph client and an offline demo.
use crate::model::*;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::PathBuf;

pub mod auth;
pub mod demo;
pub mod graph;

#[async_trait]
pub trait Backend: Send + Sync {
    async fn me(&self) -> Result<User>;
    async fn chats(&self) -> Result<Vec<Chat>>;
    /// Messages in ascending-agnostic order (the caller sorts); `cursor` loads an older page.
    async fn messages(&self, conv: &ConvRef, cursor: Option<String>) -> Result<Page>;
    /// Send a message; `thread` is the root post id when replying inside a channel thread.
    async fn send(&self, conv: &ConvRef, thread: Option<String>, out: Outgoing) -> Result<()>;
    async fn edit(&self, conv: &ConvRef, id: &MsgId, text: &str) -> Result<()>;
    async fn delete(&self, conv: &ConvRef, id: &MsgId) -> Result<()>;
    async fn react(&self, conv: &ConvRef, id: &MsgId, emoji: &str, add: bool) -> Result<()>;
    async fn mark_read(&self, chat_id: &str) -> Result<()>;
    async fn teams(&self) -> Result<Vec<Team>>;
    async fn events(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<CalEvent>>;
    async fn create_event(&self, ev: NewEvent) -> Result<()>;
    async fn rsvp(&self, event_id: &str, r: Rsvp) -> Result<()>;
    async fn presence(&self, ids: Vec<String>) -> Result<HashMap<String, Availability>>;
    /// `None` resets to automatic presence.
    async fn set_presence(&self, a: Option<Availability>) -> Result<()>;
    async fn set_status_message(&self, text: &str) -> Result<()>;
    async fn search_people(&self, q: &str) -> Result<Vec<User>>;
    async fn create_chat(&self, users: Vec<User>) -> Result<Chat>;
    async fn search_messages(&self, q: &str) -> Result<Vec<SearchHit>>;
    async fn files(&self, folder: Option<String>) -> Result<Vec<DriveItem>>;
    async fn download(&self, item: &DriveItem, dest: PathBuf) -> Result<()>;
    /// Fetch an arbitrary URL (authenticated for Graph hosts) into memory.
    async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>>;
    /// Fetch a URL to a local file (used by the external "open" path).
    async fn fetch(&self, url: &str, dest: PathBuf) -> Result<()> {
        let bytes = self.fetch_bytes(url).await?;
        tokio::task::spawn_blocking(move || std::fs::write(dest, bytes)).await??;
        Ok(())
    }
}
