//! Application panel (like Chrome DevTools): Local/Session Storage, Cookies, IndexedDB,
//! Cache Storage and usage of the page origin's storage, via CDP.
//!
//! Raw CDP commands (`Raw`) are used: chromiumoxide's types for IndexedDB/Storage
//! omit optional fields that Chrome does need (e.g. an empty `indexName` fails with -32000).

use anyhow::{Result, anyhow};
use chromiumoxide::Page;
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    /// Seconds since epoch; `None` = session cookie.
    pub expires: Option<f64>,
    pub size: i64,
    pub http_only: bool,
    pub secure: bool,
    pub same_site: String,
    pub partition_key: String,
    pub priority: String,
}

#[derive(Clone, Debug)]
pub struct IdbIndex {
    pub name: String,
    pub key_path: String,
}

#[derive(Clone, Debug)]
pub struct IdbStore {
    pub name: String,
    pub key_path: String,
    pub indexes: Vec<IdbIndex>,
}

#[derive(Clone, Debug)]
pub struct IdbDatabase {
    pub name: String,
    pub version: f64,
    pub stores: Vec<IdbStore>,
}

#[derive(Clone, Debug)]
pub struct IdbEntry {
    pub key: String,
    pub primary_key: String,
    /// Primary key as `IndexedDB.Key` (to delete the entry). `None` if it can't be expressed.
    pub primary_key_raw: Option<Value>,
    /// One-line preview (like Chrome's Value column).
    pub preview: String,
    /// Full value as indented JSON.
    pub json: String,
}

#[derive(Clone, Debug)]
pub struct CacheInfo {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub url: String,
    pub method: String,
    pub response_type: String,
    pub content_type: String,
    pub content_length: String,
    /// Seconds since epoch.
    pub time: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub usage: f64,
    pub quota: f64,
    pub breakdown: Vec<(String, f64)>,
}

/// UI request. The ones that modify something return the affected list already updated.
#[derive(Clone, Debug)]
pub enum StorageReq {
    Usage,
    ClearSiteData,
    DomItems {
        local: bool,
    },
    SetDomItem {
        local: bool,
        key: String,
        value: String,
        old_key: Option<String>,
    },
    RemoveDomItem {
        local: bool,
        key: String,
    },
    ClearDom {
        local: bool,
    },
    Cookies,
    /// Creates or edits a cookie (`old`: the one being replaced, if name/domain/path change).
    SetCookie {
        cookie: Cookie,
        old: Option<Cookie>,
    },
    DeleteCookie(Cookie),
    ClearCookies,
    IdbDatabases,
    IdbData {
        db: String,
        store: String,
        index: Option<String>,
        skip: i64,
        start: Option<String>,
    },
    IdbDeleteEntry {
        db: String,
        store: String,
        key: Value,
    },
    IdbClearStore {
        db: String,
        store: String,
    },
    IdbDeleteDb {
        db: String,
    },
    Caches,
    CacheEntries {
        cache_id: String,
    },
    DeleteCache {
        cache_id: String,
    },
    DeleteCacheEntry {
        cache_id: String,
        url: String,
    },
}

#[derive(Clone, Debug)]
pub enum StorageData {
    Usage(Usage),
    Dom {
        local: bool,
        items: Vec<(String, String)>,
    },
    Cookies(Vec<Cookie>),
    Idb(Vec<IdbDatabase>),
    IdbData {
        db: String,
        store: String,
        index: Option<String>,
        skip: i64,
        entries: Vec<IdbEntry>,
        has_more: bool,
    },
    Caches(Vec<CacheInfo>),
    CacheEntries {
        cache_id: String,
        entries: Vec<CacheEntry>,
    },
    /// The operation finished with no data to return (e.g. clearing site data).
    Done,
}

pub const IDB_PAGE: i64 = 50;

/// Generic CDP command: method + JSON params.
#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
struct Raw {
    #[serde(skip)]
    method: &'static str,
    params: Value,
}

impl chromiumoxide::Method for Raw {
    fn identifier(&self) -> chromiumoxide::types::MethodId {
        self.method.into()
    }
}

impl chromiumoxide::Command for Raw {
    type Response = Value;
}

async fn call(page: &Page, method: &'static str, params: Value) -> Result<Value> {
    Ok(page.execute(Raw { method, params }).await?.result)
}

pub async fn handle(page: &Page, origin: &str, req: StorageReq) -> Result<StorageData> {
    let dom_id = |local: bool| json!({ "securityOrigin": origin, "isLocalStorage": local });
    match req {
        StorageReq::Usage => usage(page, origin).await,
        StorageReq::ClearSiteData => {
            call(
                page,
                "Storage.clearDataForOrigin",
                json!({ "origin": origin, "storageTypes": "all" }),
            )
            .await?;
            Ok(StorageData::Done)
        }
        StorageReq::DomItems { local } => dom_items(page, origin, local).await,
        StorageReq::SetDomItem {
            local,
            key,
            value,
            old_key,
        } => {
            if let Some(old) = old_key.filter(|o| *o != key) {
                call(
                    page,
                    "DOMStorage.removeDOMStorageItem",
                    json!({ "storageId": dom_id(local), "key": old }),
                )
                .await?;
            }
            call(
                page,
                "DOMStorage.setDOMStorageItem",
                json!({ "storageId": dom_id(local), "key": key, "value": value }),
            )
            .await?;
            dom_items(page, origin, local).await
        }
        StorageReq::RemoveDomItem { local, key } => {
            call(
                page,
                "DOMStorage.removeDOMStorageItem",
                json!({ "storageId": dom_id(local), "key": key }),
            )
            .await?;
            dom_items(page, origin, local).await
        }
        StorageReq::ClearDom { local } => {
            call(
                page,
                "DOMStorage.clear",
                json!({ "storageId": dom_id(local) }),
            )
            .await?;
            dom_items(page, origin, local).await
        }
        StorageReq::Cookies => cookies(page, origin).await,
        StorageReq::SetCookie { cookie, old } => {
            if let Some(old) = old {
                delete_cookie(page, &old).await?;
            }
            let mut p = json!({
                "name": cookie.name,
                "value": cookie.value,
                "domain": cookie.domain,
                "path": if cookie.path.is_empty() { "/" } else { &cookie.path },
                "secure": cookie.secure,
                "httpOnly": cookie.http_only,
            });
            if let Some(e) = cookie.expires {
                p["expires"] = json!(e);
            }
            if !cookie.same_site.is_empty() {
                p["sameSite"] = json!(cookie.same_site);
            }
            if !cookie.priority.is_empty() {
                p["priority"] = json!(cookie.priority);
            }
            let r = call(page, "Network.setCookie", p).await?;
            if r["success"] == json!(false) {
                return Err(anyhow!("Chrome rejected the cookie"));
            }
            cookies(page, origin).await
        }
        StorageReq::DeleteCookie(c) => {
            delete_cookie(page, &c).await?;
            cookies(page, origin).await
        }
        StorageReq::ClearCookies => {
            if let StorageData::Cookies(all) = cookies(page, origin).await? {
                for c in &all {
                    delete_cookie(page, c).await?;
                }
            }
            cookies(page, origin).await
        }
        StorageReq::IdbDatabases => idb_databases(page, origin).await,
        StorageReq::IdbData {
            db,
            store,
            index,
            skip,
            start,
        } => idb_data(page, origin, db, store, index, skip, start).await,
        StorageReq::IdbDeleteEntry { db, store, key } => {
            call(
                page,
                "IndexedDB.deleteObjectStoreEntries",
                json!({
                    "securityOrigin": origin,
                    "databaseName": db,
                    "objectStoreName": store,
                    "keyRange": { "lower": key, "upper": key, "lowerOpen": false, "upperOpen": false },
                }),
            )
            .await?;
            idb_data(page, origin, db, store, None, 0, None).await
        }
        StorageReq::IdbClearStore { db, store } => {
            call(
                page,
                "IndexedDB.clearObjectStore",
                json!({ "securityOrigin": origin, "databaseName": db, "objectStoreName": store }),
            )
            .await?;
            idb_data(page, origin, db, store, None, 0, None).await
        }
        StorageReq::IdbDeleteDb { db } => {
            call(
                page,
                "IndexedDB.deleteDatabase",
                json!({ "securityOrigin": origin, "databaseName": db }),
            )
            .await?;
            idb_databases(page, origin).await
        }
        StorageReq::Caches => caches(page, origin).await,
        StorageReq::CacheEntries { cache_id } => cache_entries(page, cache_id).await,
        StorageReq::DeleteCache { cache_id } => {
            call(
                page,
                "CacheStorage.deleteCache",
                json!({ "cacheId": cache_id }),
            )
            .await?;
            caches(page, origin).await
        }
        StorageReq::DeleteCacheEntry { cache_id, url } => {
            call(
                page,
                "CacheStorage.deleteEntry",
                json!({ "cacheId": cache_id, "request": url }),
            )
            .await?;
            cache_entries(page, cache_id).await
        }
    }
}

async fn usage(page: &Page, origin: &str) -> Result<StorageData> {
    let r = call(
        page,
        "Storage.getUsageAndQuota",
        json!({ "origin": origin }),
    )
    .await?;
    let breakdown = r["usageBreakdown"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| {
            (
                b["storageType"].as_str().unwrap_or("").to_string(),
                b["usage"].as_f64().unwrap_or(0.0),
            )
        })
        .collect();
    Ok(StorageData::Usage(Usage {
        usage: r["usage"].as_f64().unwrap_or(0.0),
        quota: r["quota"].as_f64().unwrap_or(0.0),
        breakdown,
    }))
}

async fn dom_items(page: &Page, origin: &str, local: bool) -> Result<StorageData> {
    let r = call(
        page,
        "DOMStorage.getDOMStorageItems",
        json!({ "storageId": { "securityOrigin": origin, "isLocalStorage": local } }),
    )
    .await?;
    let items = r["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| Some((e[0].as_str()?.to_string(), e[1].as_str()?.to_string())))
        .collect();
    Ok(StorageData::Dom { local, items })
}

/// All the browser cookies whose domain matches the page host (like Chrome,
/// which lists the domain's and its parents' cookies, without filtering by path).
async fn cookies(page: &Page, origin: &str) -> Result<StorageData> {
    let host = origin
        .split("://")
        .nth(1)
        .unwrap_or(origin)
        .split(':')
        .next()
        .unwrap_or("")
        .to_string();
    let r = call(page, "Storage.getCookies", json!({})).await?;
    let list = r["cookies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| {
            let d = c["domain"].as_str().unwrap_or("").trim_start_matches('.');
            host == d || host.ends_with(&format!(".{d}"))
        })
        .map(|c| {
            let s = |k: &str| c[k].as_str().unwrap_or("").to_string();
            Cookie {
                name: s("name"),
                value: s("value"),
                domain: s("domain"),
                path: s("path"),
                expires: if c["session"].as_bool().unwrap_or(false) {
                    None
                } else {
                    c["expires"].as_f64()
                },
                size: c["size"].as_i64().unwrap_or(0),
                http_only: c["httpOnly"].as_bool().unwrap_or(false),
                secure: c["secure"].as_bool().unwrap_or(false),
                same_site: s("sameSite"),
                partition_key: c["partitionKey"]["topLevelSite"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                priority: s("priority"),
            }
        })
        .collect();
    Ok(StorageData::Cookies(list))
}

async fn delete_cookie(page: &Page, c: &Cookie) -> Result<()> {
    call(
        page,
        "Network.deleteCookies",
        json!({ "name": c.name, "domain": c.domain, "path": c.path }),
    )
    .await?;
    Ok(())
}

fn key_path(v: &Value) -> String {
    match v["type"].as_str() {
        Some("string") => v["string"].as_str().unwrap_or("").to_string(),
        Some("array") => v["array"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

async fn idb_databases(page: &Page, origin: &str) -> Result<StorageData> {
    let names = call(
        page,
        "IndexedDB.requestDatabaseNames",
        json!({ "securityOrigin": origin }),
    )
    .await?;
    let mut dbs = Vec::new();
    for name in names["databaseNames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| n.as_str())
    {
        let r = call(
            page,
            "IndexedDB.requestDatabase",
            json!({ "securityOrigin": origin, "databaseName": name }),
        )
        .await?;
        let d = &r["databaseWithObjectStores"];
        let stores = d["objectStores"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| IdbStore {
                name: s["name"].as_str().unwrap_or("").to_string(),
                key_path: key_path(&s["keyPath"]),
                indexes: s["indexes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|i| IdbIndex {
                        name: i["name"].as_str().unwrap_or("").to_string(),
                        key_path: key_path(&i["keyPath"]),
                    })
                    .collect(),
            })
            .collect();
        dbs.push(IdbDatabase {
            name: name.to_string(),
            version: d["version"].as_f64().unwrap_or(0.0),
            stores,
        });
    }
    Ok(StorageData::Idb(dbs))
}

async fn idb_data(
    page: &Page,
    origin: &str,
    db: String,
    store: String,
    index: Option<String>,
    skip: i64,
    start: Option<String>,
) -> Result<StorageData> {
    let mut p = json!({
        "securityOrigin": origin,
        "databaseName": db,
        "objectStoreName": store,
        "skipCount": skip,
        "pageSize": IDB_PAGE,
    });
    // Chrome fails with an empty indexName: only sent when there is an index.
    if let Some(i) = &index {
        p["indexName"] = json!(i);
    }
    // "Start from key": a number if it looks like one, otherwise text (like Chrome).
    if let Some(s) = start.filter(|s| !s.trim().is_empty()) {
        let key = match s.trim().parse::<f64>() {
            Ok(n) => json!({ "type": "number", "number": n }),
            Err(_) => json!({ "type": "string", "string": s.trim() }),
        };
        p["keyRange"] = json!({ "lower": key, "lowerOpen": false, "upperOpen": false });
    }
    let r = call(page, "IndexedDB.requestData", p).await?;
    let mut entries = Vec::new();
    for e in r["objectStoreDataEntries"].as_array().into_iter().flatten() {
        let value = &e["value"];
        let json = match value["objectId"].as_str() {
            Some(id) => {
                let r = call(
                    page,
                    "Runtime.callFunctionOn",
                    json!({
                        "objectId": id,
                        "functionDeclaration": "function(){try{return JSON.stringify(this,null,2)}catch(e){return String(this)}}",
                        "returnByValue": true,
                    }),
                )
                .await;
                let _ = call(page, "Runtime.releaseObject", json!({ "objectId": id })).await;
                r.ok()
                    .and_then(|r| r["result"]["value"].as_str().map(str::to_string))
                    .unwrap_or_default()
            }
            None => serde_json::to_string_pretty(&value["value"]).unwrap_or_default(),
        };
        entries.push(IdbEntry {
            key: remote_text(&e["key"]),
            primary_key: remote_text(&e["primaryKey"]),
            primary_key_raw: idb_key(&e["primaryKey"]),
            preview: remote_text(value),
            json,
        });
        for k in ["key", "primaryKey"] {
            if let Some(id) = e[k]["objectId"].as_str() {
                let _ = call(page, "Runtime.releaseObject", json!({ "objectId": id })).await;
            }
        }
    }
    Ok(StorageData::IdbData {
        db,
        store,
        index,
        skip,
        entries,
        has_more: r["hasMore"].as_bool().unwrap_or(false),
    })
}

/// RemoteObject → one-line text, as Chrome shows it (`{id: 1, name: 'Ann'}`).
fn remote_text(o: &Value) -> String {
    if let Some(p) = o.get("preview") {
        let props = p["properties"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|pr| {
                let v = pr["value"].as_str().unwrap_or("");
                let v = if pr["type"] == "string" {
                    format!("'{v}'")
                } else {
                    v.to_string()
                };
                if p["subtype"] == "array" {
                    v
                } else {
                    format!("{}: {v}", pr["name"].as_str().unwrap_or(""))
                }
            })
            .collect::<Vec<_>>();
        let more = if p["overflow"].as_bool().unwrap_or(false) {
            ", …"
        } else {
            ""
        };
        return if p["subtype"] == "array" {
            format!("[{}{more}]", props.join(", "))
        } else {
            format!("{{{}{more}}}", props.join(", "))
        };
    }
    match &o["value"] {
        Value::String(s) => format!("\"{s}\""),
        Value::Null => o["description"].as_str().unwrap_or("undefined").to_string(),
        v => v.to_string(),
    }
}

/// RemoteObject of a primary key → `IndexedDB.Key`. Dates and nested arrays are not supported.
fn idb_key(o: &Value) -> Option<Value> {
    match o["type"].as_str()? {
        "number" => Some(json!({ "type": "number", "number": o["value"].as_f64()? })),
        "string" => Some(json!({ "type": "string", "string": o["value"].as_str()? })),
        _ => None,
    }
}

async fn caches(page: &Page, origin: &str) -> Result<StorageData> {
    let r = call(
        page,
        "CacheStorage.requestCacheNames",
        json!({ "securityOrigin": origin }),
    )
    .await?;
    let list = r["caches"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| CacheInfo {
            id: c["cacheId"].as_str().unwrap_or("").to_string(),
            name: c["cacheName"].as_str().unwrap_or("").to_string(),
        })
        .collect();
    Ok(StorageData::Caches(list))
}

async fn cache_entries(page: &Page, cache_id: String) -> Result<StorageData> {
    let r = call(
        page,
        "CacheStorage.requestEntries",
        json!({ "cacheId": cache_id, "skipCount": 0, "pageSize": 500 }),
    )
    .await?;
    let entries = r["cacheDataEntries"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let header = |name: &str| {
                e["responseHeaders"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|h| {
                        h["name"]
                            .as_str()
                            .is_some_and(|n| n.eq_ignore_ascii_case(name))
                    })
                    .and_then(|h| h["value"].as_str())
                    .unwrap_or("")
                    .to_string()
            };
            CacheEntry {
                url: e["requestURL"].as_str().unwrap_or("").to_string(),
                method: e["requestMethod"].as_str().unwrap_or("").to_string(),
                response_type: e["responseType"].as_str().unwrap_or("").to_string(),
                content_type: header("content-type"),
                content_length: header("content-length"),
                time: e["responseTime"].as_f64().unwrap_or(0.0),
            }
        })
        .collect();
    Ok(StorageData::CacheEntries { cache_id, entries })
}
