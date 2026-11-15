# TeamsTUI

Microsoft Teams in your terminal, written in Rust. It mirrors the Teams desktop
layout — app rail on the left, list pane, conversation pane, search bar and
profile at the top — and talks to your real Teams account through Microsoft Graph.

```
  T   ‹  ›                  🔍 Search (Ctrl+E)                           ● Available   AW  Alex Wilber
                                   MB  Megan Bowen  ● Available                     📹 Video  📞 Call
   🔔1      Chat      ✎ New chat   Chat    megan.bowen@contoso.example
 Activity                         ──━━━━───────────────────────────────────────────────────────────
            MB  ● Megan Bowen
▌  💬1            are we still…    MB  Megan Bowen  5:31 AM
▌  Chat                                 Perfect, thank you! 🎉
            LC    Launch crew 🚀        📎 Launch checklist.xlsx
    👥            Diego: Demo e…
  Teams                                                                                    6:13 AM
                                                                  Yes, see you at 2 @Megan Bowen
    📅                                                                                       ❤️ 1
 Calendar                         ╭──────────────────────────────────────────────────────────────╮
                                  │ Type a message                                               │
    📞                            ╰───────────────────────────────────────────────────── Enter ➤ ╯
```

## Quick start

```sh
cargo run --release -- --demo   # sample data, no account or network needed
cargo run --release             # sign in with your work or school account
```

On first run TeamsTUI shows a device code: open the link, enter the code, sign
in. The token is cached in `~/.config/teamstui/token.json` (mode 0600) and
refreshed automatically. `teamstui --logout` forgets it.

## Features

| Area | What works |
| --- | --- |
| **Chat** | 1:1, group and meeting chats; unread markers; live updates; send, quote-reply, edit, delete; reactions; @mentions with Tab completion; attachments and links open in the browser; per-conversation drafts; load older history; new 1:1 and group chats |
| **Teams** | Teams and channels tree; posts with threaded replies; new posts, thread replies, reactions, edit, delete |
| **Activity** | Feed of new messages and @mentions; Enter jumps to the conversation |
| **Calendar** | Week grid with overlapping events, all-day row and now-line; meeting details; accept / tentative / decline; schedule a Teams meeting; join opens the meeting link |
| **Calls** | Speed dial with presence; audio/video call hand-off to Teams |
| **OneDrive** | Browse folders, open files in the browser, download |
| **Everywhere** | Search across people, chats, channels and messages; presence for you and others; set status and status message; mouse support (click, wheel); new-message toasts and terminal bell |

### What a terminal can't do

Audio, video and screen sharing cannot run in a TUI. Starting a call or joining
a meeting hands off to the Teams app / teams.microsoft.com through a deep link.
Images render as `[image]`, and rich cards as a one-line summary.

## Keys

| Key | Action |
| --- | --- |
| `Alt+1` … `Alt+6` | Activity, Chat, Teams, Calendar, Calls, OneDrive |
| `Ctrl+E` or `/` | Search |
| `Ctrl+N` or `n` | New chat (new meeting in Calendar) |
| `Ctrl+P` | Status, status message, sign out |
| `Tab` / `Shift+Tab` | Cycle list → messages → compose |
| `Enter` | Open item / send message |
| `Alt+Enter`, `Ctrl+J` | New line while composing |
| `@name` + `Tab` | Mention a participant |
| `↑` in compose | Jump into the message list |
| `r` `y` `e` `d` `o` | React, reply, edit, delete, open link (message selected) |
| `c` | Call the people in this chat |
| `←` `→` `t` `J` | Calendar: previous/next week, today, join |
| `Backspace`, `d` | OneDrive: up a folder, download |
| `Ctrl+R` | Refresh · `F1` help · `Ctrl+Q` quit |

## Signing in and permissions

By default TeamsTUI signs in with Microsoft's public "Microsoft Graph Command
Line Tools" client and requests these delegated permissions:

```
User.Read User.ReadBasic.All Chat.ReadWrite Team.ReadBasic.All Channel.ReadBasic.All
ChannelMessage.Read.All ChannelMessage.Send ChannelMessage.ReadWrite
Calendars.ReadWrite Presence.Read.All Presence.ReadWrite Files.Read.All
```

Some of these (notably `ChannelMessage.Read.All`) need **admin consent** in most
tenants. If your organisation blocks the default client or you can't get
consent, either:

- register your own app in Entra ID (public client, "Allow public client flows"
  enabled, the delegated permissions above) and pass `--client-id <id>`
  and optionally `--tenant <tenant>`; or
- request fewer scopes, e.g. chat only:
  `teamstui --scopes "User.Read Chat.ReadWrite Presence.Read.All"`.
  Views whose permission is missing show an error toast; the rest keep working.

The same settings can be given as `TEAMSTUI_CLIENT_ID`, `TEAMSTUI_TENANT` and
`TEAMSTUI_SCOPES`. Personal Microsoft accounts are not supported by the Teams
Graph APIs.

## Development

```sh
cargo test      # unit tests + an end-to-end run against the demo backend
```

`src/backend/` holds the `Backend` trait with two implementations: `graph.rs`
(Microsoft Graph) and `demo.rs` (in-memory sample data). `src/app.rs` is the
state machine, `src/ui/` the ratatui rendering.

## Releasing

Bump `version` in `Cargo.toml`, commit, then push a matching tag:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

The Release workflow builds Linux (x86_64, arm64), macOS (Intel, Apple
silicon) and Windows binaries and publishes them with checksums as a GitHub
release.
