//! OAuth 2.0 device-code sign-in against Microsoft Entra ID, with a token cache.
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::Mutex;

/// "Microsoft Graph Command Line Tools" — a Microsoft first-party public client that
/// supports the device-code flow. Override with --client-id to use your own app registration.
pub const DEFAULT_CLIENT_ID: &str = "14d82eec-204b-4c2f-b7e8-296a70dab67e";
pub const DEFAULT_TENANT: &str = "organizations";
pub const DEFAULT_SCOPES: &str = "offline_access User.Read User.ReadBasic.All Chat.ReadWrite \
Team.ReadBasic.All Channel.ReadBasic.All ChannelMessage.Read.All ChannelMessage.Send \
ChannelMessage.ReadWrite Calendars.ReadWrite Presence.Read.All Presence.ReadWrite Files.Read.All";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Token {
    client_id: String,
    tenant: String,
    scopes: String,
    access_token: String,
    refresh_token: String,
    expires_at: i64,
}

#[derive(Clone, Debug)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

pub struct Auth {
    http: reqwest::Client,
    pub client_id: String,
    pub tenant: String,
    pub scopes: String,
    token: Mutex<Token>,
}

pub fn cache_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("teamstui").join("token.json")
}

pub fn logout() -> std::io::Result<()> {
    match std::fs::remove_file(cache_path()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Auth {
    pub fn new(client_id: String, tenant: String, scopes: String) -> Self {
        let cached: Token = std::fs::read_to_string(cache_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let token = if cached.client_id == client_id && cached.tenant == tenant && cached.scopes == scopes {
            cached
        } else {
            Token::default()
        };
        Self { http: reqwest::Client::new(), client_id, tenant, scopes, token: Mutex::new(token) }
    }

    pub async fn signed_in(&self) -> bool {
        !self.token.lock().await.refresh_token.is_empty()
    }

    fn endpoint(&self, what: &str) -> String {
        format!("https://login.microsoftonline.com/{}/oauth2/v2.0/{what}", self.tenant)
    }

    async fn post(&self, what: &str, form: &[(&str, &str)]) -> Result<(bool, Value)> {
        let resp = self.http.post(self.endpoint(what)).form(form).send().await?;
        let ok = resp.status().is_success();
        let text = resp.text().await?;
        let v: Value = serde_json::from_str(&text).map_err(|_| anyhow!("unexpected sign-in response: {text}"))?;
        Ok((ok, v))
    }

    fn describe(v: &Value) -> String {
        let desc = v["error_description"].as_str().unwrap_or("");
        let first = desc.lines().next().unwrap_or("");
        format!("{}: {}", v["error"].as_str().unwrap_or("error"), first)
    }

    pub async fn start_device_flow(&self) -> Result<DeviceCode> {
        let (ok, v) = self.post("devicecode", &[("client_id", &self.client_id), ("scope", &self.scopes)]).await?;
        if !ok {
            bail!(Self::describe(&v));
        }
        Ok(DeviceCode {
            user_code: v["user_code"].as_str().unwrap_or_default().to_string(),
            verification_uri: v["verification_uri"].as_str().unwrap_or("https://microsoft.com/devicelogin").to_string(),
            device_code: v["device_code"].as_str().unwrap_or_default().to_string(),
            interval: v["interval"].as_u64().unwrap_or(5),
            expires_in: v["expires_in"].as_u64().unwrap_or(900),
        })
    }

    /// Poll until the user completes (or abandons) the sign-in in their browser.
    pub async fn poll_device_flow(&self, dc: &DeviceCode) -> Result<()> {
        let mut interval = dc.interval.max(1);
        let deadline = now() + dc.expires_in as i64;
        loop {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            if now() > deadline {
                bail!("the sign-in code expired");
            }
            let (ok, v) = self
                .post(
                    "token",
                    &[
                        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                        ("client_id", &self.client_id),
                        ("device_code", &dc.device_code),
                    ],
                )
                .await?;
            if ok {
                return self.store(&v).await;
            }
            match v["error"].as_str().unwrap_or("") {
                "authorization_pending" => {}
                "slow_down" => interval += 5,
                _ => bail!(Self::describe(&v)),
            }
        }
    }

    async fn store(&self, v: &Value) -> Result<()> {
        let mut t = self.token.lock().await;
        self.apply(&mut t, v)
    }

    fn apply(&self, t: &mut Token, v: &Value) -> Result<()> {
        let access = v["access_token"].as_str().ok_or_else(|| anyhow!("no access token in response"))?;
        t.client_id = self.client_id.clone();
        t.tenant = self.tenant.clone();
        t.scopes = self.scopes.clone();
        t.access_token = access.to_string();
        if let Some(r) = v["refresh_token"].as_str() {
            t.refresh_token = r.to_string();
        }
        t.expires_at = now() + v["expires_in"].as_i64().unwrap_or(3600);
        save(t);
        Ok(())
    }

    /// A valid access token, refreshing it when it is about to expire (or when `force`d).
    pub async fn access_token(&self, force: bool) -> Result<String> {
        let mut t = self.token.lock().await;
        if !force && !t.access_token.is_empty() && t.expires_at - now() > 120 {
            return Ok(t.access_token.clone());
        }
        if t.refresh_token.is_empty() {
            bail!("not signed in");
        }
        let refresh = t.refresh_token.clone();
        let (ok, v) = self
            .post(
                "token",
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", &self.client_id),
                    ("refresh_token", &refresh),
                    ("scope", &self.scopes),
                ],
            )
            .await?;
        if !ok {
            if v["error"] == "invalid_grant" {
                bail!("your session expired — run `teamstui --logout` and sign in again");
            }
            bail!(Self::describe(&v));
        }
        self.apply(&mut t, &v)?;
        Ok(t.access_token.clone())
    }

    /// Tenant id from the access token's `tid` claim.
    pub async fn tenant_id(&self) -> Option<String> {
        let t = self.token.lock().await;
        let payload = t.access_token.split('.').nth(1)?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
        let v: Value = serde_json::from_slice(&bytes).ok()?;
        v["tid"].as_str().map(str::to_string)
    }
}

fn save(t: &Token) {
    let path = cache_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(json) = serde_json::to_string(t) else { return };
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    if let Ok(mut f) = opts.open(&path) {
        use std::io::Write;
        let _ = f.write_all(json.as_bytes());
    }
}
