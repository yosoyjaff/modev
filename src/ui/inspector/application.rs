//! Application tab (like Chrome DevTools'): Storage, Local/Session storage,
//! IndexedDB, Cookies and Cache storage of the page origin.

use std::collections::HashSet;

use super::*;
use crate::storage::{
    CacheEntry, CacheInfo, Cookie, IDB_PAGE, IdbDatabase, IdbEntry, StorageData, StorageReq, Usage,
};

#[derive(Clone, Debug, PartialEq)]
pub enum AppSection {
    Storage,
    Dom {
        local: bool,
    },
    Cookies,
    IdbDb(String),
    IdbStore {
        db: String,
        store: String,
        index: Option<String>,
    },
    Cache(String),
}

struct IdbPage {
    skip: i64,
    entries: Vec<IdbEntry>,
    has_more: bool,
}

pub struct AppState {
    section: AppSection,
    usage: Option<Usage>,
    local: Vec<(String, String)>,
    session: Vec<(String, String)>,
    cookies: Vec<Cookie>,
    idb: Vec<IdbDatabase>,
    idb_page: Option<IdbPage>,
    caches: Vec<CacheInfo>,
    cache_entries: Option<(String, Vec<CacheEntry>)>,
    /// Index in the full list (not in the filtered one).
    selected: Option<usize>,
    /// Cell being edited: row (`None` = new row at the end) and column.
    editing: Option<(Option<usize>, usize)>,
    /// Value to load into the edit field (applied by the view, which has the `Window`).
    edit_load: Option<String>,
    collapsed: HashSet<String>,
    error: Option<String>,
    decode_cookie: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            section: AppSection::Storage,
            usage: None,
            local: Vec::new(),
            session: Vec::new(),
            cookies: Vec::new(),
            idb: Vec::new(),
            idb_page: None,
            caches: Vec::new(),
            cache_entries: None,
            selected: None,
            editing: None,
            edit_load: None,
            collapsed: HashSet::new(),
            error: None,
            decode_cookie: false,
        }
    }
}

impl AppState {
    pub fn apply(&mut self, data: StorageData) {
        self.error = None;
        match data {
            StorageData::Usage(u) => self.usage = Some(u),
            StorageData::Dom { local, items } => {
                if local {
                    self.local = items;
                } else {
                    self.session = items;
                }
            }
            StorageData::Cookies(c) => self.cookies = c,
            StorageData::Idb(dbs) => {
                self.idb = dbs;
                let gone = match &self.section {
                    AppSection::IdbDb(db) | AppSection::IdbStore { db, .. } => {
                        !self.idb.iter().any(|d| d.name == *db)
                    }
                    _ => false,
                };
                if gone {
                    self.section = AppSection::Storage;
                }
            }
            StorageData::IdbData {
                db,
                store,
                index,
                skip,
                entries,
                has_more,
            } => {
                let current = AppSection::IdbStore {
                    db: db.clone(),
                    store: store.clone(),
                    index: index.clone(),
                };
                if self.section == current {
                    self.idb_page = Some(IdbPage {
                        skip,
                        entries,
                        has_more,
                    });
                }
            }
            StorageData::Caches(c) => self.caches = c,
            StorageData::CacheEntries { cache_id, entries } => {
                self.cache_entries = Some((cache_id, entries))
            }
            StorageData::Done => {}
        }
    }

    pub fn set_error(&mut self, e: String) {
        self.error = Some(e);
    }

    fn dom(&self, local: bool) -> &[(String, String)] {
        if local { &self.local } else { &self.session }
    }
}

const COOKIE_COLS: [(&str, f32); 11] = [
    ("Name", 160.),
    ("Value", 0.),
    ("Domain", 150.),
    ("Path", 70.),
    ("Expires / Max-Age", 190.),
    ("Size", 55.),
    ("HttpOnly", 70.),
    ("Secure", 60.),
    ("SameSite", 75.),
    ("Partition Key Site", 130.),
    ("Priority", 70.),
];

fn cookie_cell(c: &Cookie, col: usize) -> String {
    let flag = |b: bool| if b { "✓".to_string() } else { String::new() };
    match col {
        0 => c.name.clone(),
        1 => c.value.clone(),
        2 => c.domain.clone(),
        3 => c.path.clone(),
        4 => c.expires.map(iso_time).unwrap_or_else(|| "Session".into()),
        5 => c.size.to_string(),
        6 => flag(c.http_only),
        7 => flag(c.secure),
        8 => c.same_site.clone(),
        9 => c.partition_key.clone(),
        _ => c.priority.clone(),
    }
}

fn iso_time(secs: f64) -> String {
    chrono::DateTime::from_timestamp_millis((secs * 1000.0) as i64)
        .map(|t| t.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_default()
}

/// `scheme://host[:port]` of the page URL.
fn origin_of(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => format!(
            "{scheme}://{}",
            rest.split(['/', '?', '#']).next().unwrap_or("")
        ),
        None => url.to_string(),
    }
}

fn matches(query: &str, fields: &[&str]) -> bool {
    query.is_empty() || fields.iter().any(|f| f.to_lowercase().contains(query))
}

impl Modev {
    fn storage(&self, req: StorageReq) {
        self.send(Command::Storage {
            origin: origin_of(&self.page_url),
            req: Box::new(req),
        });
    }

    /// Reloads the tree (IndexedDB, caches) and the visible section.
    pub(in crate::ui) fn app_reload(&mut self) {
        self.storage(StorageReq::IdbDatabases);
        self.storage(StorageReq::Caches);
        self.app_load_section();
    }

    /// The page changed Local/Session storage: refresh if it is being viewed.
    pub(in crate::ui) fn app_dom_changed(&mut self, local: bool) {
        if self.dev_tab == DevTab::Application && self.app.section == (AppSection::Dom { local }) {
            self.storage(StorageReq::DomItems { local });
        }
    }

    fn app_load_section(&mut self) {
        let req = match &self.app.section {
            AppSection::Storage => StorageReq::Usage,
            AppSection::Dom { local } => StorageReq::DomItems { local: *local },
            AppSection::Cookies => StorageReq::Cookies,
            AppSection::IdbDb(_) => StorageReq::IdbDatabases,
            AppSection::IdbStore { db, store, index } => {
                let skip = self.app.idb_page.as_ref().map(|p| p.skip).unwrap_or(0);
                StorageReq::IdbData {
                    db: db.clone(),
                    store: store.clone(),
                    index: index.clone(),
                    skip,
                    start: None,
                }
            }
            AppSection::Cache(id) => StorageReq::CacheEntries {
                cache_id: id.clone(),
            },
        };
        self.storage(req);
    }

    fn app_select(&mut self, section: AppSection) {
        if self.app.section != section {
            self.app.section = section;
            self.app.selected = None;
            self.app.editing = None;
            self.app.idb_page = None;
        }
        self.app_load_section();
    }

    pub(in crate::ui) fn app_edit_to_load(&mut self) -> Option<String> {
        self.app.edit_load.take()
    }

    fn app_begin_edit(&mut self, row: Option<usize>, col: usize, value: String) {
        self.app.selected = row;
        self.app.editing = Some((row, col));
        self.app.edit_load = Some(value);
    }

    /// Enter or losing focus in the edit field.
    pub(in crate::ui) fn app_commit_edit(&mut self, value: String) {
        let Some((row, col)) = self.app.editing.take() else {
            return;
        };
        match self.app.section.clone() {
            AppSection::Dom { local } => {
                let old = row.and_then(|i| self.app.dom(local).get(i).cloned());
                let (key, val, old_key) = match (old, col) {
                    (Some((k, v)), 0) => (value, v, Some(k)),
                    (Some((k, _)), _) => (k, value, None),
                    (None, _) => (value, String::new(), None),
                };
                if key.is_empty() {
                    return;
                }
                self.storage(StorageReq::SetDomItem {
                    local,
                    key,
                    value: val,
                    old_key,
                });
            }
            AppSection::Cookies => {
                let host = origin_of(&self.page_url)
                    .split("://")
                    .nth(1)
                    .unwrap_or("")
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .to_string();
                let old = row.and_then(|i| self.app.cookies.get(i).cloned());
                let mut c = old.clone().unwrap_or_else(|| Cookie {
                    domain: host,
                    path: "/".into(),
                    ..Default::default()
                });
                match col {
                    0 => c.name = value,
                    1 => c.value = value,
                    2 => c.domain = value,
                    3 => c.path = value,
                    _ => return,
                }
                if c.name.is_empty() || old.as_ref() == Some(&c) {
                    return;
                }
                // Name, domain and path identify the cookie: if they change, the previous one is deleted.
                let replaced =
                    old.filter(|o| (&o.name, &o.domain, &o.path) != (&c.name, &c.domain, &c.path));
                self.storage(StorageReq::SetCookie {
                    cookie: c,
                    old: replaced,
                });
            }
            _ => {}
        }
    }

    pub(in crate::ui) fn app_start_from_key(&mut self, start: String) {
        if let AppSection::IdbStore { db, store, index } = self.app.section.clone() {
            self.storage(StorageReq::IdbData {
                db,
                store,
                index,
                skip: 0,
                start: Some(start),
            });
        }
    }

    fn app_delete_selected(&mut self) {
        let Some(i) = self.app.selected.take() else {
            return;
        };
        let req = match &self.app.section {
            AppSection::Dom { local } => match self.app.dom(*local).get(i) {
                Some((k, _)) => StorageReq::RemoveDomItem {
                    local: *local,
                    key: k.clone(),
                },
                None => return,
            },
            AppSection::Cookies => match self.app.cookies.get(i) {
                Some(c) => StorageReq::DeleteCookie(c.clone()),
                None => return,
            },
            AppSection::IdbStore { db, store, .. } => {
                match self
                    .app
                    .idb_page
                    .as_ref()
                    .and_then(|p| p.entries.get(i))
                    .and_then(|e| e.primary_key_raw.clone())
                {
                    Some(key) => StorageReq::IdbDeleteEntry {
                        db: db.clone(),
                        store: store.clone(),
                        key,
                    },
                    None => return,
                }
            }
            AppSection::Cache(id) => {
                match self.app.cache_entries.as_ref().and_then(|(_, e)| e.get(i)) {
                    Some(e) => StorageReq::DeleteCacheEntry {
                        cache_id: id.clone(),
                        url: e.url.clone(),
                    },
                    None => return,
                }
            }
            _ => return,
        };
        self.storage(req);
    }

    fn app_clear_all(&mut self) {
        self.app.selected = None;
        let req = match &self.app.section {
            AppSection::Dom { local } => StorageReq::ClearDom { local: *local },
            AppSection::Cookies => StorageReq::ClearCookies,
            AppSection::IdbStore { db, store, .. } => StorageReq::IdbClearStore {
                db: db.clone(),
                store: store.clone(),
            },
            _ => return,
        };
        self.storage(req);
    }

    // ───────────────────────── Render ─────────────────────────

    pub(super) fn render_application(
        &mut self,
        inputs: &Inputs,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let main: AnyElement = match self.app.section.clone() {
            AppSection::Storage => self.app_storage_view(cx).into_any_element(),
            AppSection::Dom { local } => self.app_dom_view(local, inputs, cx).into_any_element(),
            AppSection::Cookies => self.app_cookies_view(inputs, cx).into_any_element(),
            AppSection::IdbDb(db) => self.app_idb_db_view(&db, cx).into_any_element(),
            AppSection::IdbStore { .. } => self.app_idb_store_view(inputs, cx).into_any_element(),
            AppSection::Cache(id) => self.app_cache_view(&id, inputs, cx).into_any_element(),
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .child(self.app_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(main)
                    .when_some(self.app.error.clone(), |d, e| {
                        d.child(
                            div()
                                .px(px(8.))
                                .py(px(4.))
                                .bg(rgb(ERR_BG))
                                .border_t_1()
                                .border_color(rgb(ERR_BORDER))
                                .text_color(rgb(DT_RED))
                                .child(e),
                        )
                    }),
            )
    }

    fn app_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let origin = origin_of(&self.page_url);
        let mut list = div().flex().flex_col().py(px(4.));
        let group = |label: &'static str| {
            div()
                .px(px(8.))
                .pt(px(8.))
                .pb(px(2.))
                .text_color(rgb(DT_DIM))
                .font_weight(FontWeight::SEMIBOLD)
                .child(label)
        };

        list = list.child(group("Application")).child(self.tree_item(
            "storage".into(),
            0,
            Ico::HardDrive,
            "Storage".into(),
            None,
            Some(AppSection::Storage),
            cx,
        ));
        list = list.child(group("Storage"));
        for (local, key, label) in [
            (true, "local", "Local storage"),
            (false, "session", "Session storage"),
        ] {
            let open = !self.app.collapsed.contains(key);
            list = list.child(self.tree_item(
                key.into(),
                0,
                Ico::Table,
                label.into(),
                Some(open),
                None,
                cx,
            ));
            if open {
                list = list.child(self.tree_item(
                    format!("{key}-origin"),
                    1,
                    Ico::Table,
                    origin.clone(),
                    None,
                    Some(AppSection::Dom { local }),
                    cx,
                ));
            }
        }

        let open = !self.app.collapsed.contains("idb");
        list = list.child(self.tree_item(
            "idb".into(),
            0,
            Ico::Database,
            "IndexedDB".into(),
            Some(open),
            None,
            cx,
        ));
        if open {
            for db in self.app.idb.clone() {
                let db_key = format!("idb/{}", db.name);
                let db_open = !self.app.collapsed.contains(&db_key);
                list = list.child(self.tree_item(
                    db_key.clone(),
                    1,
                    Ico::Database,
                    format!("{} - {origin}", db.name),
                    Some(db_open),
                    Some(AppSection::IdbDb(db.name.clone())),
                    cx,
                ));
                if !db_open {
                    continue;
                }
                for store in &db.stores {
                    let store_key = format!("{db_key}/{}", store.name);
                    let store_open = !self.app.collapsed.contains(&store_key);
                    let section = AppSection::IdbStore {
                        db: db.name.clone(),
                        store: store.name.clone(),
                        index: None,
                    };
                    let expand = (!store.indexes.is_empty()).then_some(store_open);
                    list = list.child(self.tree_item(
                        store_key.clone(),
                        2,
                        Ico::Table,
                        store.name.clone(),
                        expand,
                        Some(section),
                        cx,
                    ));
                    if store_open {
                        for index in &store.indexes {
                            let section = AppSection::IdbStore {
                                db: db.name.clone(),
                                store: store.name.clone(),
                                index: Some(index.name.clone()),
                            };
                            list = list.child(self.tree_item(
                                format!("{store_key}/{}", index.name),
                                3,
                                Ico::Key,
                                index.name.clone(),
                                None,
                                Some(section),
                                cx,
                            ));
                        }
                    }
                }
            }
        }

        let open = !self.app.collapsed.contains("cookies");
        list = list.child(self.tree_item(
            "cookies".into(),
            0,
            Ico::Cookie,
            "Cookies".into(),
            Some(open),
            None,
            cx,
        ));
        if open {
            list = list.child(self.tree_item(
                "cookies-origin".into(),
                1,
                Ico::Cookie,
                origin.clone(),
                None,
                Some(AppSection::Cookies),
                cx,
            ));
        }

        let open = !self.app.collapsed.contains("cache");
        list = list.child(self.tree_item(
            "cache".into(),
            0,
            Ico::Package,
            "Cache storage".into(),
            Some(open),
            None,
            cx,
        ));
        if open {
            for c in self.app.caches.clone() {
                list = list.child(self.tree_item(
                    format!("cache/{}", c.id),
                    1,
                    Ico::Package,
                    format!("{} - {origin}", c.name),
                    None,
                    Some(AppSection::Cache(c.id.clone())),
                    cx,
                ));
            }
        }

        div()
            .id("app-sidebar")
            .w(px(240.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(rgb(DT_BORDER))
            .bg(rgb(DT_TOOLBAR))
            .child(list)
    }

    /// Tree row. `expand`: `Some(open)` if it has children; `section`: what it opens on click.
    #[allow(clippy::too_many_arguments)]
    fn tree_item(
        &self,
        key: String,
        depth: usize,
        icon: Ico,
        label: String,
        expand: Option<bool>,
        section: Option<AppSection>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let active = section.as_ref() == Some(&self.app.section);
        let toggle_key = key.clone();
        div()
            .id(SharedString::from(format!("tree-{key}")))
            .h(px(22.))
            .pl(px(6. + depth as f32 * 14.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .when(active, |d| d.bg(rgb(DT_SELECTED)))
            .when(!active, |d| d.hover(|s| s.bg(rgb(DT_HOVER))))
            .child(
                div()
                    .id(SharedString::from(format!("tree-x-{key}")))
                    .size(px(14.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(DT_DIM))
                    .when_some(expand, |d, open| {
                        d.child(
                            Icon::new(if open {
                                Ico::ArrowDown
                            } else {
                                Ico::ChevronRight
                            })
                            .xsmall(),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if !this.app.collapsed.remove(&toggle_key) {
                                this.app.collapsed.insert(toggle_key.clone());
                            }
                            cx.notify();
                        }))
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(rgb(DT_DIM))
                    .child(Icon::new(icon).xsmall()),
            )
            .child(div().flex_1().min_w_0().truncate().child(label))
            .on_click(cx.listener(move |this, _, _, cx| {
                match &section {
                    Some(s) => this.app_select(s.clone()),
                    // Nodes without their own view only collapse/expand (like in Chrome).
                    None => {
                        if !this.app.collapsed.remove(&key) {
                            this.app.collapsed.insert(key.clone());
                        }
                    }
                }
                cx.notify();
            }))
    }

    fn app_toolbar(&self, inputs: Option<&Inputs>, cx: &mut Context<Self>) -> Div {
        let has_selection = self.app.selected.is_some();
        toolbar()
            .child(tool_icon(
                "app-refresh",
                Ico::Refresh,
                cx.listener(|this, _, _, cx| {
                    this.app_load_section();
                    cx.notify();
                }),
            ))
            .when_some(inputs, |d, i| {
                d.child(div().w(px(220.)).child(Input::new(&i.app_filter).xsmall()))
            })
            .child(tool_icon(
                "app-clear",
                Ico::Clear,
                cx.listener(|this, _, _, cx| {
                    this.app_clear_all();
                    cx.notify();
                }),
            ))
            .child(
                tool_icon(
                    "app-delete",
                    Ico::Close,
                    cx.listener(|this, _, _, cx| {
                        this.app_delete_selected();
                        cx.notify();
                    }),
                )
                .when(!has_selection, |d| d.opacity(0.4)),
            )
    }

    fn app_storage_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let origin = origin_of(&self.page_url);
        const COLORS: [u32; 6] = [0x4285f4, 0xea4335, 0xfbbc04, 0x34a853, 0xa142f4, 0x24c1e0];
        let mut body = div()
            .id("app-storage")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Storage"),
            )
            .child(div().text_color(rgb(DT_DIM)).child(origin))
            .child(section_title("Usage"));
        match &self.app.usage {
            Some(u) => {
                body = body.child(format!(
                    "{} used out of {} storage quota",
                    fmt_size(u.usage),
                    fmt_size(u.quota)
                ));
                let used: Vec<_> = u.breakdown.iter().filter(|(_, b)| *b > 0.0).collect();
                if u.usage > 0.0 {
                    let mut bar = div()
                        .h(px(10.))
                        .w(px(420.))
                        .flex()
                        .rounded(px(2.))
                        .overflow_hidden()
                        .bg(rgb(DT_BORDER));
                    for (i, (_, b)) in used.iter().enumerate() {
                        bar = bar.child(
                            div()
                                .h_full()
                                .w(relative((*b / u.usage) as f32))
                                .bg(rgb(COLORS[i % COLORS.len()])),
                        );
                    }
                    body = body.child(bar);
                }
                for (i, (kind, b)) in used.iter().enumerate() {
                    body = body.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .size(px(10.))
                                    .rounded(px(2.))
                                    .bg(rgb(COLORS[i % COLORS.len()])),
                            )
                            .child(div().w(px(160.)).child(kind.replace('_', " ")))
                            .child(div().text_color(rgb(DT_DIM)).child(fmt_size(*b))),
                    );
                }
            }
            None => body = body.child(div().text_color(rgb(DT_DIM)).child("Loading…")),
        }
        body.child(section_title("Clear site data")).child(
            div()
                .id("app-clear-site")
                .w(px(160.))
                .h(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .border_1()
                .border_color(rgb(DT_BORDER))
                .hover(|s| s.bg(rgb(DT_HOVER)))
                .child("Clear site data")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.storage(StorageReq::ClearSiteData);
                    this.app.local.clear();
                    this.app.session.clear();
                    this.app.cookies.clear();
                    this.app_reload();
                    cx.notify();
                })),
        )
    }

    /// Editable cell with double click (like Chrome's tables). `row`: `None` = new row.
    fn edit_cell(
        &self,
        inputs: &Inputs,
        row: Option<usize>,
        col: usize,
        value: String,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let editing = self.app.editing == Some((row, col));
        let id = SharedString::from(format!(
            "cell-{}-{col}",
            row.map(|r| r as i64).unwrap_or(-1)
        ));
        let base = div()
            .id(id)
            .h_full()
            .px(px(6.))
            .flex()
            .items_center()
            .overflow_hidden();
        if editing {
            return base.child(div().flex_1().child(Input::new(&inputs.app_edit).xsmall()));
        }
        let text = value.clone();
        base.child(div().truncate().child(text))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if ev.click_count() >= 2 {
                    this.app_begin_edit(row, col, value.clone());
                } else {
                    this.app.selected = row;
                    this.app.editing = None;
                }
                cx.notify();
            }))
    }

    fn app_dom_view(
        &mut self,
        local: bool,
        inputs: &Inputs,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query = inputs.app_filter.read(cx).value().to_lowercase();
        let items = self.app.dom(local).to_vec();
        let header = div()
            .h(px(22.))
            .flex()
            .flex_none()
            .bg(rgb(DT_TOOLBAR))
            .border_b_1()
            .border_color(rgb(DT_BORDER))
            .text_color(rgb(DT_DIM))
            .child(
                div()
                    .w(relative(0.35))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .child("Key"),
            )
            .child(
                div()
                    .flex_1()
                    .px(px(6.))
                    .border_l_1()
                    .border_color(rgb(DT_BORDER))
                    .flex()
                    .items_center()
                    .child("Value"),
            );
        let mut rows = div().flex().flex_col();
        let mut n = 0;
        for (i, (k, v)) in items.iter().enumerate() {
            if !matches(&query, &[k, v]) {
                continue;
            }
            let (copy_k, copy_v) = (k.clone(), v.clone());
            rows = rows.child(
                self.table_row(i, n)
                    .child(
                        self.edit_cell(inputs, Some(i), 0, k.clone(), cx)
                            .w(relative(0.35))
                            .flex_none(),
                    )
                    .child(
                        self.edit_cell(inputs, Some(i), 1, v.clone(), cx)
                            .flex_1()
                            .min_w_0()
                            .border_l_1()
                            .border_color(rgb(DT_BORDER)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            let items = vec![
                                ("Copy key", copy_k.clone()),
                                ("Copy value", copy_v.clone()),
                                (
                                    "Copy as JSON",
                                    serde_json::json!({ copy_k.clone(): copy_v.clone() })
                                        .to_string(),
                                ),
                            ];
                            this.copy_menu = Some((ev.position, items));
                            cx.notify();
                        }),
                    ),
            );
            n += 1;
        }
        // Empty row at the end: double click to add (like Chrome).
        rows = rows.child(
            div()
                .h(px(22.))
                .flex()
                .border_b_1()
                .border_color(rgb(DT_BORDER))
                .child(
                    self.edit_cell(inputs, None, 0, String::new(), cx)
                        .w(relative(0.35))
                        .flex_none(),
                )
                .child(div().flex_1().border_l_1().border_color(rgb(DT_BORDER))),
        );
        let preview = self
            .app
            .selected
            .and_then(|i| items.get(i))
            .map(|(_, v)| v.clone());
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.app_toolbar(Some(inputs), cx))
            .child(header)
            .child(
                div()
                    .id("app-dom-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(value_preview(preview, "Select a value to preview it"))
    }

    fn table_row(&self, i: usize, n: usize) -> Div {
        let selected = self.app.selected == Some(i);
        div()
            .h(px(22.))
            .flex()
            .flex_none()
            .border_b_1()
            .border_color(rgb(DT_BORDER))
            .when(n % 2 == 1, |d| d.bg(rgb(DT_ROW_ALT)))
            .when(selected, |d| d.bg(rgb(DT_SELECTED)))
            .when(!selected, |d| d.hover(|s| s.bg(rgb(DT_HOVER))))
    }

    fn app_cookies_view(&mut self, inputs: &Inputs, cx: &mut Context<Self>) -> impl IntoElement {
        let query = inputs.app_filter.read(cx).value().to_lowercase();
        let cookies = self.app.cookies.clone();
        let fixed: f32 = COOKIE_COLS.iter().map(|(_, w)| w).sum();
        let col = |i: usize, d: Stateful<Div>| -> Stateful<Div> {
            let w = COOKIE_COLS[i].1;
            let d = if w > 0.0 {
                d.w(px(w)).flex_none()
            } else {
                d.flex_1().min_w(px(160.))
            };
            if i > 0 {
                d.border_l_1().border_color(rgb(DT_BORDER))
            } else {
                d
            }
        };

        let mut header = div()
            .h(px(22.))
            .flex()
            .flex_none()
            .bg(rgb(DT_TOOLBAR))
            .border_b_1()
            .border_color(rgb(DT_BORDER))
            .text_color(rgb(DT_DIM));
        for (i, (label, _)) in COOKIE_COLS.iter().enumerate() {
            header = header.child(col(
                i,
                div()
                    .id(("ch", i))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .child(div().truncate().child(*label)),
            ));
        }

        let mut rows = div().flex().flex_col();
        let mut n = 0;
        for (i, c) in cookies.iter().enumerate() {
            if !matches(&query, &[&c.name, &c.value, &c.domain, &c.path]) {
                continue;
            }
            let mut row = self.table_row(i, n);
            for k in 0..COOKIE_COLS.len() {
                let text = cookie_cell(c, k);
                let cell = if k <= 3 {
                    self.edit_cell(inputs, Some(i), k, text, cx)
                } else {
                    div()
                        .id(SharedString::from(format!("cc-{i}-{k}")))
                        .h_full()
                        .px(px(6.))
                        .flex()
                        .items_center()
                        .overflow_hidden()
                        .child(div().truncate().child(text))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.app.selected = Some(i);
                            this.app.editing = None;
                            cx.notify();
                        }))
                };
                row = row.child(col(k, cell));
            }
            let (name, value) = (c.name.clone(), c.value.clone());
            rows = rows.child(row.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    let items = vec![
                        ("Copy name", name.clone()),
                        ("Copy value", value.clone()),
                        ("Copy as header", format!("{name}={value}")),
                    ];
                    this.copy_menu = Some((ev.position, items));
                    cx.notify();
                }),
            ));
            n += 1;
        }
        let mut new_row = div()
            .h(px(22.))
            .flex()
            .border_b_1()
            .border_color(rgb(DT_BORDER));
        new_row = new_row.child(col(0, self.edit_cell(inputs, None, 0, String::new(), cx)));

        let decode = self.app.decode_cookie;
        let preview = self.app.selected.and_then(|i| cookies.get(i)).map(|c| {
            if decode {
                percent_decode(&c.value)
            } else {
                c.value.clone()
            }
        });
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.app_toolbar(Some(inputs), cx))
            .child(
                div()
                    .id("app-cookies")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .child(
                        div()
                            .min_w(px(fixed + 160.))
                            .flex()
                            .flex_col()
                            .child(header)
                            .child(rows)
                            .child(new_row),
                    ),
            )
            .child(
                div()
                    .h(px(26.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(12.))
                    .px(px(8.))
                    .border_t_1()
                    .border_color(rgb(DT_BORDER))
                    .bg(rgb(DT_TOOLBAR))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Cookie Value"),
                    )
                    .child(checkbox(
                        "app-decode",
                        "Show URL-decoded",
                        decode,
                        cx.listener(|this, _, _, cx| {
                            this.app.decode_cookie = !this.app.decode_cookie;
                            cx.notify();
                        }),
                    )),
            )
            .child(value_preview(preview, "Select a cookie to view its value"))
    }

    fn app_idb_db_view(&mut self, db: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let origin = origin_of(&self.page_url);
        let info = self.app.idb.iter().find(|d| d.name == db).cloned();
        let row = |k: &'static str, v: String| {
            div()
                .flex()
                .py(px(2.))
                .child(div().w(px(140.)).text_color(rgb(DT_DIM)).child(k))
                .child(v)
        };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .px(px(12.))
                .h(px(26.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .border_1()
                .border_color(rgb(DT_BORDER))
                .hover(|s| s.bg(rgb(DT_HOVER)))
                .child(label)
        };
        let name = db.to_string();
        div()
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(db.to_string()),
            )
            .child(row("Security origin", origin))
            .when_some(info, |d, i| {
                d.child(row("Version", format!("{}", i.version)))
                    .child(row("Object stores", i.stores.len().to_string()))
            })
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .gap(px(8.))
                    .child(
                        button("idb-delete", "Delete database")
                            .text_color(rgb(DT_RED))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.storage(StorageReq::IdbDeleteDb { db: name.clone() });
                                this.app.section = AppSection::Storage;
                                cx.notify();
                            })),
                    )
                    .child(
                        button("idb-refresh", "Refresh database").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.storage(StorageReq::IdbDatabases);
                                cx.notify();
                            },
                        )),
                    ),
            )
    }

    fn app_idb_store_view(&mut self, inputs: &Inputs, cx: &mut Context<Self>) -> impl IntoElement {
        let AppSection::IdbStore { db, store, index } = self.app.section.clone() else {
            return div();
        };
        let meta = self
            .app
            .idb
            .iter()
            .find(|d| d.name == db)
            .and_then(|d| d.stores.iter().find(|s| s.name == store))
            .cloned();
        let store_path = meta
            .as_ref()
            .map(|s| s.key_path.clone())
            .unwrap_or_default();
        let index_path = index.as_ref().and_then(|i| {
            meta.as_ref()?
                .indexes
                .iter()
                .find(|x| x.name == *i)
                .map(|x| x.key_path.clone())
        });
        let page = self.app.idb_page.as_ref();
        let (skip, has_more) = page.map(|p| (p.skip, p.has_more)).unwrap_or((0, false));
        let entries: Vec<IdbEntry> = page.map(|p| p.entries.clone()).unwrap_or_default();

        let key_label = |path: &str| {
            if path.is_empty() {
                "Key".to_string()
            } else {
                format!("Key (Key path: \"{path}\")")
            }
        };
        let page_req = move |skip: i64| StorageReq::IdbData {
            db: db.clone(),
            store: store.clone(),
            index: index.clone(),
            skip,
            start: None,
        };
        let (prev_req, next_req) = (page_req(skip - IDB_PAGE), page_req(skip + IDB_PAGE));

        let toolbar = self
            .app_toolbar(None, cx)
            .child(separator())
            .child(
                div()
                    .w(px(160.))
                    .child(Input::new(&inputs.idb_start).xsmall()),
            )
            .child(separator())
            .child(
                tool_icon(
                    "idb-prev",
                    Ico::ChevronLeft,
                    cx.listener(move |this, _, _, cx| {
                        if skip > 0 {
                            this.storage(prev_req.clone());
                        }
                        cx.notify();
                    }),
                )
                .when(skip == 0, |d| d.opacity(0.4)),
            )
            .child(
                tool_icon(
                    "idb-next",
                    Ico::ChevronRight,
                    cx.listener(move |this, _, _, cx| {
                        if has_more {
                            this.storage(next_req.clone());
                        }
                        cx.notify();
                    }),
                )
                .when(!has_more, |d| d.opacity(0.4)),
            )
            .child(div().text_color(rgb(DT_DIM)).child(if entries.is_empty() {
                String::new()
            } else {
                format!("{}–{}", skip + 1, skip + entries.len() as i64)
            }));

        let head = |text: String| {
            div()
                .px(px(6.))
                .flex()
                .items_center()
                .overflow_hidden()
                .child(div().truncate().child(text))
        };
        let mut header = div()
            .h(px(22.))
            .flex()
            .flex_none()
            .bg(rgb(DT_TOOLBAR))
            .border_b_1()
            .border_color(rgb(DT_BORDER))
            .text_color(rgb(DT_DIM))
            .child(head("#".into()).w(px(50.)).flex_none());
        match &index_path {
            Some(ip) => {
                header = header
                    .child(
                        head(key_label(ip))
                            .w(px(200.))
                            .flex_none()
                            .border_l_1()
                            .border_color(rgb(DT_BORDER)),
                    )
                    .child(
                        head(format!("Primary key (Key path: \"{store_path}\")"))
                            .w(px(200.))
                            .flex_none()
                            .border_l_1()
                            .border_color(rgb(DT_BORDER)),
                    )
            }
            None => {
                header = header.child(
                    head(key_label(&store_path))
                        .w(px(200.))
                        .flex_none()
                        .border_l_1()
                        .border_color(rgb(DT_BORDER)),
                )
            }
        }
        header = header.child(
            head("Value".into())
                .flex_1()
                .border_l_1()
                .border_color(rgb(DT_BORDER)),
        );

        let mut rows = div().flex().flex_col();
        for (i, e) in entries.iter().enumerate() {
            let mut row = self
                .table_row(i, i)
                .id(("idb-row", i))
                .child(
                    cell_flex((skip + i as i64).to_string())
                        .w(px(50.))
                        .flex_none()
                        .flex_grow_0(),
                )
                .child(cell(e.key.clone(), 200.));
            if index_path.is_some() {
                row = row.child(cell(e.primary_key.clone(), 200.));
            }
            let json = e.json.clone();
            let key = e.primary_key.clone();
            rows = rows.child(
                row.child(
                    cell_flex(e.preview.clone())
                        .border_l_1()
                        .border_color(rgb(DT_BORDER))
                        .font_family(MONO_FONT),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.app.selected = Some(i);
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.copy_menu = Some((
                            ev.position,
                            vec![("Copy key", key.clone()), ("Copy value", json.clone())],
                        ));
                        cx.notify();
                    }),
                ),
            );
        }
        if entries.is_empty() {
            rows = rows.child(div().p(px(12.)).text_color(rgb(DT_DIM)).child("No data"));
        }
        let preview = self
            .app
            .selected
            .and_then(|i| entries.get(i))
            .map(|e| e.json.clone());
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(header)
            .child(
                div()
                    .id("app-idb-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .child(value_preview(preview, "Select a value to preview it"))
    }

    fn app_cache_view(
        &mut self,
        cache_id: &str,
        inputs: &Inputs,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let query = inputs.app_filter.read(cx).value().to_lowercase();
        let entries = match &self.app.cache_entries {
            Some((id, e)) if id == cache_id => e.clone(),
            _ => Vec::new(),
        };
        let id = cache_id.to_string();
        let toolbar = self
            .app_toolbar(Some(inputs), cx)
            .child(separator())
            .child(tool_icon(
                "cache-delete",
                Ico::Delete,
                cx.listener(move |this, _, _, cx| {
                    this.storage(StorageReq::DeleteCache {
                        cache_id: id.clone(),
                    });
                    this.app.section = AppSection::Storage;
                    this.app_load_section();
                    cx.notify();
                }),
            ));
        const COLS: [(&str, f32); 5] = [
            ("Response-Type", 110.),
            ("Content-Type", 170.),
            ("Content-Length", 110.),
            ("Time Cached", 190.),
            ("Method", 70.),
        ];
        let mut header = div()
            .h(px(22.))
            .flex()
            .flex_none()
            .bg(rgb(DT_TOOLBAR))
            .border_b_1()
            .border_color(rgb(DT_BORDER))
            .text_color(rgb(DT_DIM))
            .child(cell("#", 50.).border_l_0())
            .child(cell_flex("Name").border_l_1().border_color(rgb(DT_BORDER)));
        for (label, w) in COLS {
            header = header.child(cell(label, w));
        }
        let mut rows = div().flex().flex_col();
        let mut n = 0;
        for (i, e) in entries.iter().enumerate() {
            if !matches(&query, &[&e.url]) {
                continue;
            }
            let url = e.url.clone();
            rows = rows.child(
                self.table_row(i, n)
                    .id(("cache-row", i))
                    .child(cell(i.to_string(), 50.).border_l_0())
                    .child(
                        cell_flex(e.url.clone())
                            .border_l_1()
                            .border_color(rgb(DT_BORDER)),
                    )
                    .child(cell(e.response_type.clone(), COLS[0].1))
                    .child(cell(e.content_type.clone(), COLS[1].1))
                    .child(cell(e.content_length.clone(), COLS[2].1))
                    .child(cell(clock(e.time, false), COLS[3].1))
                    .child(cell(e.method.clone(), COLS[4].1))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.app.selected = Some(i);
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                            this.copy_menu = Some((ev.position, vec![("Copy URL", url.clone())]));
                            cx.notify();
                        }),
                    ),
            );
            n += 1;
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(header)
            .child(
                div()
                    .id("app-cache-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(rows),
            )
    }
}

/// Bottom panel with the selected value (formatted JSON if it is).
fn value_preview(value: Option<String>, empty: &'static str) -> Div {
    let body = match value {
        Some(v) => code_view(pretty_json(&v), "", true).into_any_element(),
        None => div()
            .text_color(rgb(DT_DIM))
            .child(empty)
            .into_any_element(),
    };
    div()
        .h(px(180.))
        .flex_none()
        .border_t_1()
        .border_color(rgb(DT_BORDER))
        .child(
            div()
                .id("app-preview")
                .size_full()
                .overflow_y_scroll()
                .p(px(8.))
                .child(body),
        )
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // Byte-level hex check: slicing `s` could split a UTF-8 char and panic,
        // and `from_str_radix` would accept a sign (`%+1`).
        if bytes[i] == b'%'
            && let [_, h, l, ..] = bytes[i..]
            && let (Some(h), Some(l)) = ((h as char).to_digit(16), (l as char).to_digit(16))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::{origin_of, percent_decode};

    #[test]
    fn origin_of_strips_path_query_and_fragment() {
        assert_eq!(
            origin_of("http://localhost:8100/tabs/1?x=1"),
            "http://localhost:8100"
        );
        assert_eq!(origin_of("https://a.dev#top"), "https://a.dev");
        assert_eq!(origin_of("about:blank"), "about:blank");
    }

    #[test]
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%20b+c"), "a b c");
        assert_eq!(percent_decode("%7B%22k%22%3A1%7D"), "{\"k\":1}");
        assert_eq!(percent_decode("caf%C3%A9"), "café");
        // Invalid or truncated escapes are kept as-is.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
        assert_eq!(percent_decode("%aé"), "%aé");
        assert_eq!(percent_decode("%+1"), "% 1");
    }
}
