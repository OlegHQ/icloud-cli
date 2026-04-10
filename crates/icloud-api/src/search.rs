use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use chrono::{Local, NaiveDate, NaiveDateTime};
use hex::encode as hex_encode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, SchemaBuilder, Value, STORED, STRING, TEXT,
};
use tantivy::{Index, IndexReader, TantivyDocument, Term};

use crate::error::{Error, Result};
use crate::hme::{HideMyEmailClient, HmeAlias};
use crate::notes::markdown::{disambiguate_filename, title_to_filename_stem};
use crate::notes::NotesSyncEngine;
use crate::reminders::SyncEngine;

const SEARCH_SCHEMA_VERSION: u32 = 2;
const INDEX_WRITER_HEAP_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchService {
    Notes,
    Reminders,
    HideMyEmail,
}

impl SearchService {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notes => "notes",
            Self::Reminders => "reminders",
            Self::HideMyEmail => "hide_my_email",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchResultKind {
    NoteFolder,
    NoteFile,
    ReminderList,
    ReminderFile,
    HideMyEmailAlias,
}

impl SearchResultKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::NoteFolder => "note_folder",
            Self::NoteFile => "note_file",
            Self::ReminderList => "reminder_list",
            Self::ReminderFile => "reminder_file",
            Self::HideMyEmailAlias => "hide_my_email_alias",
        }
    }

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "note_folder" => Ok(Self::NoteFolder),
            "note_file" => Ok(Self::NoteFile),
            "reminder_list" => Ok(Self::ReminderList),
            "reminder_file" => Ok(Self::ReminderFile),
            "hide_my_email_alias" => Ok(Self::HideMyEmailAlias),
            other => Err(Error::Search(format!("unknown indexed kind '{other}'"))),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub score: f32,
    pub service: SearchService,
    pub kind: SearchResultKind,
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub query: String,
    pub limit: usize,
    pub scopes: Vec<String>,
    pub service: Option<SearchService>,
}

#[derive(Debug, Clone)]
pub struct SearchIndex {
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchMeta {
    version: u32,
    fingerprint: String,
    #[serde(default)]
    cache_updated_at: Option<String>,
}

#[derive(Debug, Clone)]
struct IndexedDocument {
    id: String,
    service: SearchService,
    kind: SearchResultKind,
    path: String,
    name: String,
    container: Option<String>,
    due: Option<String>,
    content: String,
    scopes: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct SearchFields {
    doc_id: Field,
    service: Field,
    kind: Field,
    path: Field,
    name: Field,
    container: Field,
    due: Field,
    content: Field,
    scope: Field,
}

impl SearchFields {
    fn build() -> (Schema, Self) {
        let mut builder = SchemaBuilder::default();
        let doc_id = builder.add_text_field("doc_id", STRING | STORED);
        let service = builder.add_text_field("service", STRING | STORED);
        let kind = builder.add_text_field("kind", STRING | STORED);
        let path = builder.add_text_field("path", TEXT | STORED);
        let name = builder.add_text_field("name", TEXT | STORED);
        let container = builder.add_text_field("container", TEXT | STORED);
        let due = builder.add_text_field("due", STRING | STORED);
        let content = builder.add_text_field("content", TEXT | STORED);
        let scope = builder.add_text_field("scope", STRING);
        let schema = builder.build();
        (
            schema,
            Self {
                doc_id,
                service,
                kind,
                path,
                name,
                container,
                due,
                content,
                scope,
            },
        )
    }

    fn from_schema(schema: &Schema) -> Result<Self> {
        let field = |name: &str| {
            schema
                .get_field(name)
                .map_err(|_| Error::Search(format!("missing search field '{name}'")))
        };
        Ok(Self {
            doc_id: field("doc_id")?,
            service: field("service")?,
            kind: field("kind")?,
            path: field("path")?,
            name: field("name")?,
            container: field("container")?,
            due: field("due")?,
            content: field("content")?,
            scope: field("scope")?,
        })
    }
}

impl SearchIndex {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { root: path.into() }
    }

    pub async fn refresh(
        &self,
        notes: Option<&mut NotesSyncEngine>,
        reminders: Option<&mut SyncEngine>,
        force: bool,
    ) -> Result<usize> {
        let mut total = 0;

        if let Some(notes) = notes {
            total += self.refresh_notes(notes, force).await?;
        }

        if let Some(reminders) = reminders {
            total += self.refresh_reminders(reminders, force)?;
        }

        Ok(total)
    }

    pub fn search(
        &self,
        services: &[SearchService],
        options: &SearchOptions,
    ) -> Result<Vec<SearchHit>> {
        if options.query.trim().is_empty() {
            return Err(Error::Usage("search query cannot be empty".into()));
        }

        let mut unique_services = Vec::with_capacity(services.len());
        for service in services {
            if !unique_services.contains(service) {
                unique_services.push(*service);
            }
        }

        let mut hits = Vec::new();
        for service in unique_services {
            if options.service.is_some_and(|selected| selected != service) {
                continue;
            }
            hits.extend(self.search_service(service, options)?);
        }

        let limit = options.limit.max(1);
        hits.sort_by(compare_hits);
        hits.truncate(limit);
        Ok(hits)
    }

    async fn collect_note_documents(
        &self,
        notes: &mut NotesSyncEngine,
    ) -> Result<Vec<IndexedDocument>> {
        self.hydrate_note_search_texts(notes).await?;

        let mut docs = Vec::new();

        let mut note_folder_names: Vec<_> = notes.cache.folders.values().cloned().collect();
        note_folder_names.sort();
        for folder_name in &note_folder_names {
            let path = format!("/Notes/{folder_name}");
            docs.push(IndexedDocument {
                id: format!("folder:{folder_name}"),
                service: SearchService::Notes,
                kind: SearchResultKind::NoteFolder,
                path: path.clone(),
                name: folder_name.clone(),
                container: None,
                due: None,
                content: format!("{folder_name}\n{path}"),
                scopes: scope_paths(&path),
            });
        }

        for folder_name in note_folder_names {
            for entry in note_entries(notes, &folder_name) {
                let Some(note) = notes.cache.notes.get(&entry.id) else {
                    continue;
                };
                let body = note
                    .body_markdown
                    .clone()
                    .or_else(|| note.search_text.clone())
                    .unwrap_or_default();
                let path = format!("/Notes/{folder_name}/{}", entry.filename);
                let snippet = if note.snippet.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", note.snippet)
                };
                docs.push(IndexedDocument {
                    id: note_path_doc_id(SearchService::Notes, &entry.id),
                    service: SearchService::Notes,
                    kind: SearchResultKind::NoteFile,
                    path: path.clone(),
                    name: note.title.clone(),
                    container: Some(folder_name.clone()),
                    due: None,
                    content: format!("{}\n{folder_name}\n{path}{snippet}\n{body}", note.title),
                    scopes: scope_paths(&path),
                });
            }
        }

        docs.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(docs)
    }

    async fn refresh_notes(&self, notes: &mut NotesSyncEngine, force: bool) -> Result<usize> {
        let cache_updated_at = notes.cache.updated_at.clone();
        if self.can_skip_refresh(SearchService::Notes, cache_updated_at.as_deref(), force)? {
            return Ok(0);
        }

        let docs = self.collect_note_documents(notes).await?;
        self.refresh_service(
            SearchService::Notes,
            &docs,
            force,
            cache_updated_at.as_deref(),
        )
    }

    fn collect_reminder_documents(&self, reminders: &SyncEngine) -> Vec<IndexedDocument> {
        let mut docs = Vec::new();
        let mut reminder_lists: Vec<_> = reminders.cache.lists.values().cloned().collect();
        reminder_lists.sort();
        for list_name in &reminder_lists {
            let path = format!("/Reminders/{list_name}");
            docs.push(IndexedDocument {
                id: format!("list:{list_name}"),
                service: SearchService::Reminders,
                kind: SearchResultKind::ReminderList,
                path: path.clone(),
                name: list_name.clone(),
                container: None,
                due: None,
                content: format!("{list_name}\n{path}"),
                scopes: scope_paths(&path),
            });
        }

        for list_name in reminder_lists {
            for entry in reminder_entries(reminders, &list_name) {
                let Some(reminder) = reminders.cache.reminders.get(&entry.id) else {
                    continue;
                };
                let path = format!("/Reminders/{list_name}/{}", entry.filename);
                let notes_text = reminder.notes.clone().unwrap_or_default();
                let due = reminder.due.clone().unwrap_or_default();
                docs.push(IndexedDocument {
                    id: note_path_doc_id(SearchService::Reminders, &entry.id),
                    service: SearchService::Reminders,
                    kind: SearchResultKind::ReminderFile,
                    path: path.clone(),
                    name: reminder.title.clone(),
                    container: Some(list_name.clone()),
                    due: reminder.due.clone(),
                    content: format!(
                        "{}\n{list_name}\n{path}\n{notes_text}\n{due}",
                        reminder.title
                    ),
                    scopes: scope_paths(&path),
                });
            }
        }

        docs.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.id.cmp(&right.id))
        });
        docs
    }

    fn refresh_reminders(&self, reminders: &SyncEngine, force: bool) -> Result<usize> {
        let cache_updated_at = reminders.cache.updated_at.clone();
        if self.can_skip_refresh(SearchService::Reminders, cache_updated_at.as_deref(), force)? {
            return Ok(0);
        }

        let docs = self.collect_reminder_documents(reminders);
        self.refresh_service(
            SearchService::Reminders,
            &docs,
            force,
            cache_updated_at.as_deref(),
        )
    }

    /// Refresh the HideMyEmail index. Unlike notes/reminders there's no local
    /// cache file with an `updated_at` stamp — the fingerprint of the current
    /// alias list is the only freshness signal, so we always fetch and let
    /// `refresh_service` skip the rebuild when nothing changed.
    pub async fn refresh_hme(
        &self,
        client: &HideMyEmailClient,
        force: bool,
    ) -> Result<usize> {
        let aliases = client.list_aliases_parsed().await?;
        let docs = collect_hme_documents(&aliases);
        self.refresh_service(SearchService::HideMyEmail, &docs, force, None)
    }

    async fn hydrate_note_search_texts(&self, notes: &mut NotesSyncEngine) -> Result<()> {
        let mut note_ids: Vec<String> = notes
            .cache
            .notes
            .iter()
            .filter(|(_, note)| !note.deleted)
            .filter(|(_, note)| note.body_markdown.is_none() && note.search_text.is_none())
            .map(|(id, _)| id.clone())
            .collect();
        note_ids.sort();
        notes.hydrate_search_texts(&note_ids).await
    }

    fn refresh_service(
        &self,
        service: SearchService,
        docs: &[IndexedDocument],
        force: bool,
        cache_updated_at: Option<&str>,
    ) -> Result<usize> {
        let fingerprint = fingerprint(docs);
        let meta = self.read_meta(service).ok();

        if !force
            && meta.as_ref().is_some_and(|meta| {
                meta.version == SEARCH_SCHEMA_VERSION
                    && meta.fingerprint == fingerprint
                    && meta.cache_updated_at.as_deref() == cache_updated_at
            })
        {
            return Ok(docs.len());
        }

        self.rebuild_index(service, docs, &fingerprint, cache_updated_at)?;
        Ok(docs.len())
    }

    fn can_skip_refresh(
        &self,
        service: SearchService,
        cache_updated_at: Option<&str>,
        force: bool,
    ) -> Result<bool> {
        if force {
            return Ok(false);
        }

        let Some(meta) = self.read_meta(service).ok() else {
            return Ok(false);
        };
        Ok(meta.version == SEARCH_SCHEMA_VERSION
            && meta.cache_updated_at.as_deref() == cache_updated_at)
    }

    fn search_service(
        &self,
        service: SearchService,
        options: &SearchOptions,
    ) -> Result<Vec<SearchHit>> {
        let index = self.open_index(service)?;
        let fields = SearchFields::from_schema(&index.schema())?;
        let reader = self.reader(&index)?;
        let searcher = reader.searcher();

        let mut parser = QueryParser::for_index(
            &index,
            vec![fields.name, fields.container, fields.path, fields.content],
        );
        parser.set_conjunction_by_default();

        let text_query: Box<dyn Query> = parser
            .parse_query(&options.query)
            .map_err(|e| Error::Usage(format!("invalid search query: {e}")))?;

        let mut clauses: Vec<(Occur, Box<dyn Query>)> = vec![(Occur::Must, text_query)];

        if !options.scopes.is_empty() {
            let scope_terms: Vec<(Occur, Box<dyn Query>)> = options
                .scopes
                .iter()
                .map(|scope| {
                    let term = Term::from_field_text(fields.scope, scope);
                    (
                        Occur::Should,
                        Box::new(TermQuery::new(term, IndexRecordOption::Basic)) as Box<dyn Query>,
                    )
                })
                .collect();
            clauses.push((Occur::Must, Box::new(BooleanQuery::new(scope_terms))));
        }

        let query: Box<dyn Query> = if clauses.len() == 1 {
            clauses.pop().unwrap().1
        } else {
            Box::new(BooleanQuery::new(clauses))
        };

        let limit = options.limit.max(1);
        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .map_err(search_err)?;

        let mut hits = Vec::with_capacity(top_docs.len());
        for (score, address) in top_docs {
            let document: TantivyDocument = searcher.doc(address).map_err(search_err)?;
            let kind = SearchResultKind::from_str(
                field_text(&document, fields.kind)
                    .as_deref()
                    .ok_or_else(|| Error::Search("indexed search document missing kind".into()))?,
            )?;
            let content = field_text(&document, fields.content).unwrap_or_default();
            hits.push(SearchHit {
                score,
                service,
                kind,
                path: field_text(&document, fields.path)
                    .ok_or_else(|| Error::Search("indexed search document missing path".into()))?,
                name: field_text(&document, fields.name)
                    .ok_or_else(|| Error::Search("indexed search document missing name".into()))?,
                container: field_text(&document, fields.container)
                    .filter(|value| !value.is_empty()),
                due: field_text(&document, fields.due).filter(|value| !value.is_empty()),
                snippet: build_snippet(&options.query, &content),
                id: field_text(&document, fields.doc_id)
                    .ok_or_else(|| Error::Search("indexed search document missing id".into()))?,
            });
        }

        Ok(hits)
    }

    fn open_index(&self, service: SearchService) -> Result<Index> {
        let root = self.service_root(service);
        fs::create_dir_all(&root)?;
        let (schema, _) = SearchFields::build();
        let directory = MmapDirectory::open(&root).map_err(search_err)?;
        Index::open_or_create(directory, schema).map_err(search_err)
    }

    fn rebuild_index(
        &self,
        service: SearchService,
        docs: &[IndexedDocument],
        fingerprint: &str,
        cache_updated_at: Option<&str>,
    ) -> Result<()> {
        let root = self.service_root(service);
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(&root)?;

        let index = self.open_index(service)?;
        let fields = SearchFields::from_schema(&index.schema())?;
        let mut writer = index.writer(INDEX_WRITER_HEAP_BYTES).map_err(search_err)?;
        writer.delete_all_documents().map_err(search_err)?;

        for doc in docs {
            let mut indexed = TantivyDocument::default();
            indexed.add_text(fields.doc_id, &doc.id);
            indexed.add_text(fields.service, doc.service.as_str());
            indexed.add_text(fields.kind, doc.kind.as_str());
            indexed.add_text(fields.path, &doc.path);
            indexed.add_text(fields.name, &doc.name);
            indexed.add_text(
                fields.container,
                doc.container.as_deref().unwrap_or_default(),
            );
            indexed.add_text(fields.due, doc.due.as_deref().unwrap_or_default());
            indexed.add_text(fields.content, &doc.content);
            for scope in &doc.scopes {
                indexed.add_text(fields.scope, scope);
            }
            writer.add_document(indexed).map_err(search_err)?;
        }

        writer.commit().map_err(search_err)?;
        writer.wait_merging_threads().map_err(search_err)?;

        let meta = SearchMeta {
            version: SEARCH_SCHEMA_VERSION,
            fingerprint: fingerprint.to_string(),
            cache_updated_at: cache_updated_at.map(ToOwned::to_owned),
        };
        fs::write(
            self.meta_path(service),
            serde_json::to_vec_pretty(&meta).map_err(|e| Error::Search(e.to_string()))?,
        )?;
        Ok(())
    }

    fn reader(&self, index: &Index) -> Result<IndexReader> {
        index.reader().map_err(search_err)
    }

    fn service_root(&self, service: SearchService) -> PathBuf {
        self.root.join(service.as_str())
    }

    fn meta_path(&self, service: SearchService) -> PathBuf {
        self.service_root(service).join("icloud-search-state.json")
    }

    fn read_meta(&self, service: SearchService) -> Result<SearchMeta> {
        let bytes = fs::read(self.meta_path(service))?;
        serde_json::from_slice(&bytes).map_err(|e| Error::Search(e.to_string()))
    }
}

#[derive(Debug, Clone)]
struct NamedEntry {
    id: String,
    filename: String,
}

fn build_disambiguated_entries<I>(items: I) -> Vec<NamedEntry>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut items: Vec<(String, String)> = items.into_iter().collect();
    items.sort_by(|(left_id, left_title), (right_id, right_title)| {
        left_title
            .to_ascii_lowercase()
            .cmp(&right_title.to_ascii_lowercase())
            .then_with(|| left_title.cmp(right_title))
            .then_with(|| left_id.cmp(right_id))
    });

    let mut existing = HashSet::with_capacity(items.len());
    items
        .into_iter()
        .map(|(id, title)| {
            let stem = title_to_filename_stem(&title);
            let filename = disambiguate_filename(&stem, &existing);
            existing.insert(filename.to_ascii_lowercase());
            NamedEntry { id, filename }
        })
        .collect()
}

fn note_entries(notes: &NotesSyncEngine, folder_name: &str) -> Vec<NamedEntry> {
    let Some(folder_id) = notes.cache.find_folder_by_name(folder_name) else {
        return Vec::new();
    };

    build_disambiguated_entries(notes.cache.notes.iter().filter_map(|(id, note)| {
        (!note.deleted && note.folder_ref.as_deref() == Some(folder_id.as_str()))
            .then(|| (id.clone(), note.title.clone()))
    }))
}

fn reminder_entries(reminders: &SyncEngine, list_name: &str) -> Vec<NamedEntry> {
    let Some(list_id) = reminders.cache.find_list_by_name(list_name) else {
        return Vec::new();
    };

    build_disambiguated_entries(
        reminders
            .cache
            .reminders
            .iter()
            .filter_map(|(id, reminder)| {
                (reminder.list_ref.as_deref() == Some(list_id.as_str()))
                    .then(|| (id.clone(), reminder.title.clone()))
            }),
    )
}

fn note_path_doc_id(service: SearchService, id: &str) -> String {
    format!("{}:{id}", service.as_str())
}

fn collect_hme_documents(aliases: &[HmeAlias]) -> Vec<IndexedDocument> {
    let mut existing = HashSet::with_capacity(aliases.len());
    let mut docs: Vec<IndexedDocument> = aliases
        .iter()
        .map(|alias| {
            let base = if alias.label.trim().is_empty() {
                alias
                    .hme
                    .split('@')
                    .next()
                    .unwrap_or("alias")
                    .to_string()
            } else {
                alias.label.clone()
            };
            let stem = title_to_filename_stem(&base);
            let filename = disambiguate_filename(&stem, &existing);
            existing.insert(filename.to_ascii_lowercase());
            let path = format!("/HideMyEmail/{filename}");
            let name = if alias.label.trim().is_empty() {
                alias.hme.clone()
            } else {
                alias.label.clone()
            };
            let forward = alias.forward_to_email.clone().unwrap_or_default();
            // Pack every user-visible field into `content` so a query on the
            // label / email / note / forwarding address lands a hit.
            let content = format!(
                "{name}\n{hme}\n{label}\n{note}\n{forward}\n{path}",
                hme = alias.hme,
                label = alias.label,
                note = alias.note,
            );
            IndexedDocument {
                id: format!("hme:{}", alias.anonymous_id),
                service: SearchService::HideMyEmail,
                kind: SearchResultKind::HideMyEmailAlias,
                path: path.clone(),
                name,
                container: None,
                due: None,
                content,
                scopes: scope_paths(&path),
            }
        })
        .collect();
    docs.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.id.cmp(&right.id))
    });
    docs
}

fn scope_paths(path: &str) -> Vec<String> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return vec!["/".to_string()];
    }

    let mut scopes = vec!["/".to_string()];
    let mut current = String::new();
    for part in trimmed.split('/') {
        current.push('/');
        current.push_str(part);
        scopes.push(current.clone());
    }
    scopes
}

fn fingerprint(docs: &[IndexedDocument]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(SEARCH_SCHEMA_VERSION.to_le_bytes());
    for doc in docs {
        hasher.update(doc.id.as_bytes());
        hasher.update([0]);
        hasher.update(doc.service.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(doc.kind.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(doc.path.as_bytes());
        hasher.update([0]);
        hasher.update(doc.name.as_bytes());
        hasher.update([0]);
        hasher.update(doc.container.as_deref().unwrap_or_default().as_bytes());
        hasher.update([0]);
        hasher.update(doc.due.as_deref().unwrap_or_default().as_bytes());
        hasher.update([0]);
        hasher.update(doc.content.as_bytes());
        hasher.update([0]);
    }
    hex_encode(hasher.finalize())
}

fn compare_hits(left: &SearchHit, right: &SearchHit) -> Ordering {
    hit_bucket(left)
        .cmp(&hit_bucket(right))
        .then_with(|| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| left.path.cmp(&right.path))
        .then_with(|| left.id.cmp(&right.id))
}

fn hit_bucket(hit: &SearchHit) -> u8 {
    match (hit.service, hit.kind, hit.due.as_deref()) {
        (SearchService::Reminders, SearchResultKind::ReminderFile, Some(due)) => {
            match reminder_due_bucket(due) {
                ReminderDueBucket::Future => 0,
                ReminderDueBucket::Other => 1,
                ReminderDueBucket::Past => 2,
            }
        }
        (SearchService::Reminders, SearchResultKind::ReminderFile, None) => 1,
        _ => 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ReminderDueBucket {
    Future,
    Other,
    Past,
}

fn reminder_due_bucket(due: &str) -> ReminderDueBucket {
    let now = Local::now();
    if let Ok(value) = NaiveDateTime::parse_from_str(due, "%Y-%m-%d %H:%M") {
        return if value >= now.naive_local() {
            ReminderDueBucket::Future
        } else {
            ReminderDueBucket::Past
        };
    }

    if let Ok(date) = NaiveDate::parse_from_str(due, "%Y-%m-%d") {
        return if date >= now.date_naive() {
            ReminderDueBucket::Future
        } else {
            ReminderDueBucket::Past
        };
    }

    ReminderDueBucket::Other
}

fn field_text(doc: &TantivyDocument, field: Field) -> Option<String> {
    doc.get_first(field)
        .and_then(|value| value.as_value().as_str())
        .map(ToString::to_string)
}

fn build_snippet(query: &str, haystack: &str) -> Option<String> {
    let content = haystack.trim();
    if content.is_empty() {
        return None;
    }

    let lowercase = content.to_ascii_lowercase();
    let query = query.trim().to_ascii_lowercase();

    if let Some(snippet) = excerpt_for_match(content, &lowercase, &query) {
        return Some(snippet);
    }

    for term in query
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|term| !term.is_empty())
    {
        if let Some(snippet) = excerpt_for_match(content, &lowercase, term) {
            return Some(snippet);
        }
    }

    Some(truncate_for_snippet(content))
}

fn excerpt_for_match(original: &str, lowercase: &str, needle: &str) -> Option<String> {
    let start = lowercase.find(needle)?;
    let snippet_start = start.saturating_sub(48);
    let snippet_end = (start + needle.len() + 96).min(original.len());
    let mut snippet = original[snippet_start..snippet_end].trim().to_string();
    if snippet_start > 0 {
        snippet.insert_str(0, "...");
    }
    if snippet_end < original.len() {
        snippet.push_str("...");
    }
    Some(snippet.replace('\n', " "))
}

fn truncate_for_snippet(value: &str) -> String {
    let collapsed = value.replace('\n', " ");
    if collapsed.chars().count() <= 120 {
        return collapsed;
    }

    let mut out = String::new();
    for ch in collapsed.chars().take(117) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn search_err(error: impl std::fmt::Display) -> Error {
    Error::Search(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scopes_cover_ancestors() {
        assert_eq!(
            scope_paths("/Notes/Work/Quarterly Review.md"),
            vec![
                "/".to_string(),
                "/Notes".to_string(),
                "/Notes/Work".to_string(),
                "/Notes/Work/Quarterly Review.md".to_string(),
            ]
        );
    }

    #[test]
    fn snippet_prefers_match() {
        let snippet = build_snippet(
            "quarterly",
            "Roadmap\nQuarterly review agenda for the automation project\nNext steps",
        )
        .unwrap();
        assert!(snippet.to_ascii_lowercase().contains("quarterly"));
    }

    #[test]
    fn search_roundtrip_filters_by_scope() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path());
        let docs = vec![
            IndexedDocument {
                id: "notes:note-1".into(),
                service: SearchService::Notes,
                kind: SearchResultKind::NoteFile,
                path: "/Notes/Work/Quarterly Review.md".into(),
                name: "Quarterly Review".into(),
                container: Some("Work".into()),
                due: None,
                content: "Quarterly review agenda and action items".into(),
                scopes: scope_paths("/Notes/Work/Quarterly Review.md"),
            },
            IndexedDocument {
                id: "reminders:rem-1".into(),
                service: SearchService::Reminders,
                kind: SearchResultKind::ReminderFile,
                path: "/Reminders/Home/Buy Milk.md".into(),
                name: "Buy Milk".into(),
                container: Some("Home".into()),
                due: Some("2030-01-02".into()),
                content: "Buy milk and bread this evening".into(),
                scopes: scope_paths("/Reminders/Home/Buy Milk.md"),
            },
        ];
        let note_docs = vec![docs[0].clone()];
        let reminder_docs = vec![docs[1].clone()];
        let notes_fingerprint = fingerprint(&note_docs);
        let reminders_fingerprint = fingerprint(&reminder_docs);
        index
            .rebuild_index(SearchService::Notes, &note_docs, &notes_fingerprint, None)
            .unwrap();
        index
            .rebuild_index(
                SearchService::Reminders,
                &reminder_docs,
                &reminders_fingerprint,
                None,
            )
            .unwrap();

        let hits = index
            .search(
                &[SearchService::Notes],
                &SearchOptions {
                    query: "quarterly".into(),
                    limit: 10,
                    scopes: vec!["/Notes/Work".into()],
                    service: Some(SearchService::Notes),
                },
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "/Notes/Work/Quarterly Review.md");

        let filtered_out = index
            .search(
                &[SearchService::Notes],
                &SearchOptions {
                    query: "quarterly".into(),
                    limit: 10,
                    scopes: vec!["/Notes/Personal".into()],
                    service: Some(SearchService::Notes),
                },
            )
            .unwrap();
        assert!(filtered_out.is_empty());
    }

    #[test]
    fn search_prioritizes_upcoming_reminders() {
        let dir = tempdir().unwrap();
        let index = SearchIndex::new(dir.path());
        let docs = vec![
            IndexedDocument {
                id: "reminders:future".into(),
                service: SearchService::Reminders,
                kind: SearchResultKind::ReminderFile,
                path: "/Reminders/Home/Future.md".into(),
                name: "Future".into(),
                container: Some("Home".into()),
                due: Some("2030-01-02".into()),
                content: "project follow up".into(),
                scopes: scope_paths("/Reminders/Home/Future.md"),
            },
            IndexedDocument {
                id: "reminders:past".into(),
                service: SearchService::Reminders,
                kind: SearchResultKind::ReminderFile,
                path: "/Reminders/Home/Past.md".into(),
                name: "Past".into(),
                container: Some("Home".into()),
                due: Some("2020-01-02".into()),
                content: "project follow up".into(),
                scopes: scope_paths("/Reminders/Home/Past.md"),
            },
        ];
        let reminder_fingerprint = fingerprint(&docs);
        index
            .rebuild_index(SearchService::Reminders, &docs, &reminder_fingerprint, None)
            .unwrap();

        let hits = index
            .search(
                &[SearchService::Reminders],
                &SearchOptions {
                    query: "project".into(),
                    limit: 10,
                    scopes: vec![],
                    service: Some(SearchService::Reminders),
                },
            )
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].due.as_deref(), Some("2030-01-02"));
        assert_eq!(hits[1].due.as_deref(), Some("2020-01-02"));
    }
}
