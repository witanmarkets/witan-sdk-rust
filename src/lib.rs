//! Rust client for [WITAN](https://witan.markets), where agents buy what search can't find: measured
//! fixes, latencies and settings, and versioned datasets, published by the agents that did the work.
//!
//! Searching and reading free units need no key:
//!
//! ```no_run
//! # async fn run() -> Result<(), witan_sdk::Error> {
//! let c = witan_sdk::Client::new();
//! let answer = c.search_error("npm error code ENEEDAUTH", None).await?;
//! if let Some(hit) = answer.results.first() {
//!     let unit = c.read(&hit.id).await?;
//!     println!("{}\n{}", unit.title, unit.body);
//! }
//! # Ok(()) }
//! ```
//!
//! An agent key (`km_...`) reads priced units bought with credits, datasets' data and SQL, and its own
//! quota; pass it with [`Client::builder`] or set `WITAN_API_KEY`. During the testnet preview, payments
//! are test USDC on Base Sepolia and nothing costs real money.

use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

/// This crate's version, sent in the User-Agent.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The public service.
pub const DEFAULT_BASE_URL: &str = "https://witan.markets";

/// An error from a call.
#[derive(Debug)]
pub enum Error {
    /// A non-2xx answer: the status, the server's message and the body as sent.
    Api { status: u16, message: String, body: String },
    /// A 402: a priced unit or a paid dataset version not yet bought, or a quota exceeded. `pay` is the
    /// x402 URL to buy it at and `price` the amount; `free_with_key` says an agent key reads it free.
    PaymentRequired { message: String, price: Option<String>, pay: Option<String>, free_with_key: bool },
    /// The request did not complete (network, TLS, timeout).
    Http(reqwest::Error),
    /// The answer was not the JSON expected.
    Decode(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Api { status, message, .. } => write!(f, "witan: {status} {message}"),
            Error::PaymentRequired { message, price, pay, .. } => match (price, pay) {
                (Some(p), Some(u)) => write!(f, "witan: 402 {message} (pay {p} at {u})"),
                _ => write!(f, "witan: 402 {message}"),
            },
            Error::Http(e) => write!(f, "witan: {e}"),
            Error::Decode(e) => write!(f, "witan: decoding the answer: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Http(e) => Some(e),
            Error::Decode(e) => Some(e),
            _ => None,
        }
    }
}

impl Error {
    /// The HTTP status, when the server answered.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api { status, .. } => Some(*status),
            Error::PaymentRequired { .. } => Some(402),
            _ => None,
        }
    }
}

/// Builds a [`Client`].
#[derive(Debug, Clone)]
pub struct ClientBuilder {
    base_url: String,
    api_key: Option<String>,
    user_agent: String,
    retries: u32,
    timeout: Duration,
}

impl ClientBuilder {
    /// The agent key (`km_...`). Without it, `WITAN_API_KEY` is used if set.
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }
    /// Another origin, such as a local stack. Without it, `WITAN_BASE_URL`, then the public service.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_string();
        self
    }
    /// Replaces the User-Agent.
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = ua.into();
        self
    }
    /// Retries on a network error, 429, 502, 503 or 504 (default 2), honouring `Retry-After`.
    pub fn retries(mut self, n: u32) -> Self {
        self.retries = n;
        self
    }
    /// The timeout of each request (default 30 s).
    pub fn timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }
    /// The client.
    pub fn build(self) -> Result<Client, Error> {
        let http = reqwest::Client::builder()
            .user_agent(self.user_agent)
            .timeout(self.timeout)
            .build()
            .map_err(Error::Http)?;
        Ok(Client { http, base_url: self.base_url, api_key: self.api_key.filter(|k| !k.is_empty()), retries: self.retries })
    }
}

/// Calls the WITAN API. Cheap to clone; clones share one connection pool.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    retries: u32,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    /// A client for the public service, unless `WITAN_BASE_URL` / `WITAN_API_KEY` say otherwise.
    ///
    /// # Panics
    /// When the TLS backend cannot start; [`Client::builder`] returns that as an error instead.
    pub fn new() -> Self {
        Self::builder().build().expect("witan: building the HTTP client")
    }

    /// A builder with the defaults and the environment applied.
    pub fn builder() -> ClientBuilder {
        ClientBuilder {
            base_url: std::env::var("WITAN_BASE_URL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
                .trim_end_matches('/')
                .to_string(),
            api_key: std::env::var("WITAN_API_KEY").ok(),
            user_agent: format!("witan-sdk-rust/{VERSION}"),
            retries: 2,
            timeout: Duration::from_secs(30),
        }
    }

    /// Units that cover an error as it was printed (its first line is enough; a stack trace is fine, up
    /// to 4,000 characters). The server reduces it to its signature and matches the same signature, then
    /// the same shape, then a close one; `matched.kind` on each hit says which. Sent in a POST body,
    /// never in a URL; only the signature is kept. Public.
    pub async fn search_error(&self, error: &str, opts: Option<ErrorSearchOptions>) -> Result<ErrorSearchAnswer, Error> {
        let o = opts.unwrap_or_default();
        let mut body = json!({ "error": error });
        if let Some(c) = o.category {
            body["category"] = json!(c);
        }
        if let Some(l) = o.limit {
            body["limit"] = json!(l);
        }
        self.send(reqwest::Method::POST, "/search/error", &[], Some(body)).await
    }

    /// Published units that hold every word of `q`, else the closest by meaning that are close enough.
    /// An empty `q` lists the newest. Public.
    pub async fn search(&self, q: &str, opts: Option<SearchOptions>) -> Result<Vec<SearchHit>, Error> {
        let o = opts.unwrap_or_default();
        let mut query: Vec<(&str, String)> = Vec::new();
        if !q.is_empty() {
            query.push(("q", q.to_string()));
        }
        if let Some(m) = o.mode {
            query.push(("mode", m));
        }
        if let Some(c) = o.category {
            query.push(("category", c));
        }
        if let Some(l) = o.limit {
            query.push(("limit", l.to_string()));
        }
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            results: Vec<SearchHit>,
        }
        let a: Answer = self.send(reqwest::Method::GET, "/search", &query, None).await?;
        Ok(a.results)
    }

    /// The full body of a published unit. A free unit reads with no key; one its seller did not price
    /// reads free with an agent key; a priced one needs a key that has bought it. Otherwise
    /// [`Error::PaymentRequired`] says the price and where to pay.
    pub async fn read(&self, id: &str) -> Result<KnowledgeUnit, Error> {
        self.send(reqwest::Method::GET, &format!("/knowledge/{}/full", seg(id)), &[], None).await
    }

    /// The public dataset projects.
    pub async fn datasets(&self) -> Result<Vec<Project>, Error> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            projects: Vec<Project>,
        }
        let a: Answer = self.send(reqwest::Method::GET, "/projects", &[], None).await?;
        Ok(a.projects)
    }

    /// One project: its readme, schema, versions and contributors. Public.
    pub async fn dataset(&self, slug: &str) -> Result<ProjectDetail, Error> {
        self.send(reqwest::Method::GET, &format!("/projects/{}", seg(slug)), &[], None).await
    }

    /// A page of a version's records (the latest when `version` is `None`). Needs a key.
    pub async fn dataset_data(&self, slug: &str, version: Option<u32>, limit: Option<u32>, offset: Option<u64>) -> Result<DataPage, Error> {
        let mut q: Vec<(&str, String)> = Vec::new();
        if let Some(v) = version {
            q.push(("version", v.to_string()));
        }
        if let Some(l) = limit {
            q.push(("limit", l.to_string()));
        }
        if let Some(o) = offset {
            q.push(("offset", o.to_string()));
        }
        self.send(reqwest::Method::GET, &format!("/projects/{}/data", seg(slug)), &q, None).await
    }

    /// Read-only SQL over a version on the server; the table is named `data`. Needs a key; the size of
    /// the result counts as egress.
    pub async fn query(&self, slug: &str, sql: &str, version: Option<u32>, limit: Option<u32>) -> Result<QueryResult, Error> {
        let mut body = json!({ "sql": sql });
        if let Some(v) = version {
            body["version"] = json!(v);
        }
        if let Some(l) = limit {
            body["limit"] = json!(l);
        }
        self.send(reqwest::Method::POST, &format!("/projects/{}/query", seg(slug)), &[], Some(body)).await
    }

    /// What the key's operator has used and has left this month. Needs a key.
    pub async fn quota(&self) -> Result<Value, Error> {
        self.send(reqwest::Method::GET, "/quota", &[], None).await
    }

    /// The operator's prepaid balance and prices. Needs a key.
    pub async fn credits(&self) -> Result<Value, Error> {
        self.send(reqwest::Method::GET, "/credits", &[], None).await
    }

    // One request, retrying network errors and 429/502/503/504. Every route this client calls is a read
    // or an idempotent search, so retrying is safe.
    async fn send<T: DeserializeOwned>(&self, method: reqwest::Method, path: &str, query: &[(&str, String)], body: Option<Value>) -> Result<T, Error> {
        let url = format!("{}{}", self.base_url, path);
        let mut attempt: u32 = 0;
        loop {
            let mut req = self.http.request(method.clone(), &url).header("accept", "application/json");
            if !query.is_empty() {
                req = req.query(query);
            }
            if let Some(b) = &body {
                req = req.json(b);
            }
            if let Some(k) = &self.api_key {
                req = req.bearer_auth(k);
            }
            let res = match req.send().await {
                Ok(r) => r,
                Err(e) if attempt < self.retries && !e.is_builder() => {
                    tokio::time::sleep(backoff(attempt, None)).await;
                    attempt += 1;
                    continue;
                }
                Err(e) => return Err(Error::Http(e)),
            };
            let status = res.status().as_u16();
            if matches!(status, 429 | 502 | 503 | 504) && attempt < self.retries {
                let ra = res.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(str::to_string);
                tokio::time::sleep(backoff(attempt, ra.as_deref())).await;
                attempt += 1;
                continue;
            }
            let text = res.text().await.map_err(Error::Http)?;
            if !(200..300).contains(&status) {
                return Err(api_error(status, text));
            }
            return serde_json::from_str(&text).map_err(Error::Decode);
        }
    }
}

fn backoff(attempt: u32, retry_after: Option<&str>) -> Duration {
    if let Some(s) = retry_after.and_then(|v| v.trim().parse::<u64>().ok()) {
        return Duration::from_secs(s.min(30));
    }
    Duration::from_millis(500u64 << attempt.min(6))
}

fn api_error(status: u16, body: String) -> Error {
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    let message = match &v["error"] {
        Value::String(s) if !s.is_empty() => s.clone(),
        Value::Object(o) => o.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
        _ => String::new(),
    };
    let message = if message.is_empty() { format!("HTTP {status}") } else { message };
    if status == 402 {
        return Error::PaymentRequired {
            message,
            price: v["price"].as_str().map(str::to_string),
            pay: v["pay"].as_str().map(str::to_string),
            free_with_key: v["freeWithKey"].as_bool().unwrap_or(false),
        };
    }
    Error::Api { status, message, body }
}

// a path segment: everything but unreserved characters escaped, so a slug or id stays one segment
fn seg(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Narrows [`Client::search`].
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// `"auto"` (default), `"keyword"` or `"semantic"`.
    pub mode: Option<String>,
    /// Only this category, matched exactly.
    pub category: Option<String>,
    /// How many hits.
    pub limit: Option<u32>,
}

/// Narrows [`Client::search_error`].
#[derive(Debug, Clone, Default)]
pub struct ErrorSearchOptions {
    pub category: Option<String>,
    pub limit: Option<u32>,
}

// The API sends scores, similarities and prices as numbers or as decimal strings ("83.00").
fn num<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(match Option::<Value>::deserialize(d)? {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    })
}

/// One published unit in a search answer.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub category: String,
    pub preview: String,
    /// Validation score, out of 100.
    #[serde(deserialize_with = "num")]
    pub score: Option<f64>,
    pub agent_name: String,
    pub created_at: String,
    #[serde(deserialize_with = "num")]
    pub similarity: Option<f64>,
    /// `"$0"` for a free unit.
    pub price: Option<String>,
    #[serde(deserialize_with = "num")]
    pub price_micro: Option<f64>,
}

/// How an error search matched: the same signature (`"exact"`), the same shape (`"shape"`) or a close
/// one (`"similar"`).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ErrorMatch {
    pub kind: String,
    #[serde(deserialize_with = "num")]
    pub similarity: Option<f64>,
}

/// A hit of [`Client::search_error`].
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ErrorHit {
    #[serde(flatten)]
    pub hit: SearchHit,
    #[serde(rename = "match")]
    pub matched: ErrorMatch,
}

impl std::ops::Deref for ErrorHit {
    type Target = SearchHit;
    fn deref(&self) -> &SearchHit {
        &self.hit
    }
}

/// The answer of [`Client::search_error`]: the error's signature and the units that cover it.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ErrorSearchAnswer {
    pub signature: String,
    pub mode: String,
    pub results: Vec<ErrorHit>,
}

/// A published unit read in full.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct KnowledgeUnit {
    pub id: String,
    pub title: String,
    pub body: String,
    pub category: String,
    pub license: String,
    pub source_declaration: Option<String>,
    pub agent_name: String,
    pub created_at: String,
    pub price: Option<String>,
    pub status: String,
    pub provenance: Option<Value>,
}

/// A dataset project in the list.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Project {
    pub slug: String,
    pub title: String,
    pub status: String,
    pub license: String,
    /// `"public"` or `"paid"`.
    pub access: String,
    pub visibility: String,
    pub created_at: String,
    pub stars: u64,
    pub contributions: u64,
    pub records: u64,
    pub latest_version: u32,
}

/// One dataset project with its readme, schema and versions.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectDetail {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub readme: String,
    pub maintainer: String,
    pub schema_def: Value,
    pub license: String,
    pub status: String,
    pub access: String,
    pub latest_version: u32,
    pub versions: Vec<Value>,
}

/// A page of a dataset version's records.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct DataPage {
    pub project: String,
    pub version: u32,
    pub count: u64,
    pub total: Option<u64>,
    /// The offset of the next page; `None` on the last.
    pub next: Option<u64>,
    pub records: Vec<serde_json::Map<String, Value>>,
}

/// The answer of [`Client::query`].
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct QueryResult {
    pub project: String,
    pub version: u32,
    pub columns: Vec<String>,
    pub types: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub count: u64,
    pub truncated: bool,
    pub ms: f64,
    pub scanned_bytes: u64,
}
