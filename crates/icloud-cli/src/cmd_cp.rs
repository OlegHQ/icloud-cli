use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bashbox::fs::types::MkdirOptions;
use bashbox::fs::FileSystem;
use bashbox::InMemoryFs;
use icloud_api::hme::HideMyEmailClient;
use icloud_api::notes::{NotesStore, NotesSyncEngine};
use icloud_api::reminders::{RemindersStore, SyncEngine};
use icloud_api::session::{load_session, SecretsBackend};
use icloud_bash::pathmap::{classify, normalize_vpath, VfsTarget};
use icloud_bash::ICloudFs;
use tokio::sync::Mutex;

use crate::output::{print_ok_with, OutputMode};
use crate::{IcloudFsArgs, OpenNotes, OpenReminders};

pub(crate) async fn run_cp(
    out: OutputMode,
    secrets: SecretsBackend,
    max_age: u64,
    args: IcloudFsArgs,
    recursive: bool,
    src: &str,
    dest: &str,
) -> icloud_api::Result<()> {
    let src = Endpoint::parse(src)?;
    let dest = Endpoint::parse(dest)?;

    match (&src, &dest) {
        (Endpoint::Local(_), Endpoint::Local(_)) | (Endpoint::Icloud(_), Endpoint::Icloud(_)) => {
            return Err(icloud_api::Error::Usage(
                "`icloud cp` requires exactly one `icloud:/...` endpoint".into(),
            ));
        }
        _ => {}
    }

    let mut vfs = VfsContext::open(secrets, max_age, args).await?;
    let copied = match (&src, &dest) {
        (Endpoint::Local(local), Endpoint::Icloud(remote)) => {
            copy_local_to_icloud(&mut vfs, local, remote, recursive).await?
        }
        (Endpoint::Icloud(remote), Endpoint::Local(local)) => {
            copy_icloud_to_local(&mut vfs, remote, local, recursive).await?
        }
        _ => unreachable!(),
    };

    vfs.save().await?;
    print_ok_with(
        out.json,
        &format!("Copied {copied} file(s)"),
        &serde_json::json!({
            "count": copied,
            "src": src.display(),
            "dest": dest.display(),
        }),
    );
    Ok(())
}

struct VfsContext {
    notes_store: NotesStore,
    notes_engine: Arc<Mutex<NotesSyncEngine>>,
    reminders_store: RemindersStore,
    reminders_engine: Arc<Mutex<SyncEngine>>,
    fs: Arc<ICloudFs>,
}

impl VfsContext {
    async fn open(
        secrets: SecretsBackend,
        max_age: u64,
        args: IcloudFsArgs,
    ) -> icloud_api::Result<Self> {
        let session_path = args.session_path();
        let notes_db = args.notes_db_path();
        let reminders_db = args.reminders_db_path();

        let notes =
            OpenNotes::open(&session_path, &notes_db, secrets, false, max_age, true).await?;
        let reminders =
            OpenReminders::open(&session_path, &reminders_db, secrets, false, max_age, true)
                .await?;

        let session = load_session(&session_path, secrets)?;
        let hme = HideMyEmailClient::new(session)?;
        let OpenNotes {
            store: notes_store,
            engine: notes_engine,
        } = notes;
        let OpenReminders {
            store: reminders_store,
            engine: reminders_engine,
        } = reminders;
        let notes_engine = Arc::new(Mutex::new(notes_engine));
        let reminders_engine = Arc::new(Mutex::new(reminders_engine));

        let fs = Arc::new(
            ICloudFs::new(
                Arc::new(InMemoryFs::new()),
                notes_engine.clone(),
                reminders_engine.clone(),
                Arc::new(hme),
            )
            .await,
        );

        Ok(Self {
            notes_store,
            notes_engine,
            reminders_store,
            reminders_engine,
            fs,
        })
    }

    async fn save(&mut self) -> icloud_api::Result<()> {
        let mut notes = self.notes_engine.lock().await;
        self.notes_store.save_cache(&mut notes.cache)?;
        drop(notes);
        let mut reminders = self.reminders_engine.lock().await;
        self.reminders_store.save_cache(&mut reminders.cache)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum Endpoint {
    Local(LocalSpec),
    Icloud(IcloudSpec),
}

impl Endpoint {
    fn parse(raw: &str) -> icloud_api::Result<Self> {
        if raw.trim_start().starts_with("icloud:") {
            return Ok(Self::Icloud(IcloudSpec::parse(raw)?));
        }
        Ok(Self::Local(LocalSpec {
            raw: raw.to_string(),
            path: PathBuf::from(raw),
        }))
    }

    fn display(&self) -> String {
        match self {
            Endpoint::Local(local) => local.raw.clone(),
            Endpoint::Icloud(remote) => format!("icloud:{}", remote.normalized),
        }
    }
}

#[derive(Clone, Debug)]
struct LocalSpec {
    raw: String,
    path: PathBuf,
}

#[derive(Clone, Debug)]
struct IcloudSpec {
    raw: String,
    normalized: String,
    target: VfsTarget,
}

impl IcloudSpec {
    fn parse(raw: &str) -> icloud_api::Result<Self> {
        let trimmed = raw.trim();
        let without_scheme = trimmed.strip_prefix("icloud:").unwrap_or(trimmed).trim();
        let normalized = normalize_vpath(without_scheme);
        let target = classify(&normalized)
            .map_err(|_| icloud_api::Error::Usage(format!("invalid iCloud path '{raw}'")))?;
        if matches!(target, VfsTarget::Passthrough) {
            return Err(icloud_api::Error::Usage(format!(
                "path '{raw}' is outside the iCloud VFS"
            )));
        }
        Ok(Self {
            raw: raw.to_string(),
            normalized,
            target,
        })
    }

    fn join(&self, child: &str) -> icloud_api::Result<Self> {
        let joined = if self.normalized == "/" {
            format!("/{child}")
        } else {
            format!("{}/{}", self.normalized, child)
        };
        Self::parse(&format!("icloud:{joined}"))
    }

    fn basename(&self) -> Option<&str> {
        self.normalized
            .rsplit('/')
            .find(|segment| !segment.is_empty())
    }
}

async fn copy_local_to_icloud(
    vfs: &mut VfsContext,
    src: &LocalSpec,
    dest: &IcloudSpec,
    recursive: bool,
) -> icloud_api::Result<usize> {
    let metadata = std::fs::metadata(&src.path)?;
    if metadata.is_dir() {
        if !recursive {
            return Err(icloud_api::Error::Usage(
                "copying a directory requires --recursive".into(),
            ));
        }
        return copy_local_dir_to_icloud(vfs, &src.path, dest).await;
    }

    let target = resolve_icloud_file_destination(dest, &src.path)?;
    ensure_import_destination(vfs, &target).await?;
    import_local_file(vfs, &src.path, &target).await?;
    Ok(1)
}

async fn copy_local_dir_to_icloud(
    vfs: &mut VfsContext,
    src_dir: &Path,
    dest: &IcloudSpec,
) -> icloud_api::Result<usize> {
    match &dest.target {
        VfsTarget::NotesRoot => {
            import_local_tree_to_service_root(vfs, src_dir, dest, SearchRoot::Notes).await
        }
        VfsTarget::RemindersRoot => {
            import_local_tree_to_service_root(vfs, src_dir, dest, SearchRoot::Reminders).await
        }
        VfsTarget::NotesFolder { .. } | VfsTarget::RemindersList { .. } => {
            ensure_import_destination(vfs, dest).await?;
            import_local_leaf_dir(vfs, src_dir, dest).await
        }
        _ => Err(icloud_api::Error::Usage(format!(
            "cannot copy a host directory into '{}'",
            dest.raw
        ))),
    }
}

async fn import_local_tree_to_service_root(
    vfs: &mut VfsContext,
    src_dir: &Path,
    dest: &IcloudSpec,
    root: SearchRoot,
) -> icloud_api::Result<usize> {
    let mut copied = 0usize;
    for entry in std::fs::read_dir(src_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = os_to_string(entry.file_name())?;
        if !file_type.is_dir() {
            return Err(icloud_api::Error::Usage(format!(
                "copying into '{}' requires immediate subdirectories that map to {}",
                dest.raw,
                root.collection_label()
            )));
        }

        let collection_dest = dest.join(&name)?;
        ensure_import_destination(vfs, &collection_dest).await?;
        copied += import_local_leaf_dir(vfs, &entry.path(), &collection_dest).await?;
    }
    Ok(copied)
}

async fn import_local_leaf_dir(
    vfs: &mut VfsContext,
    src_dir: &Path,
    dest: &IcloudSpec,
) -> icloud_api::Result<usize> {
    let mut copied = 0usize;
    for entry in std::fs::read_dir(src_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            return Err(icloud_api::Error::Usage(format!(
                "nested directories are not valid inside '{}'",
                dest.raw
            )));
        }
        if !file_type.is_file() {
            continue;
        }

        let name = os_to_string(entry.file_name())?;
        if !name.ends_with(".md") {
            return Err(icloud_api::Error::Usage(format!(
                "only .md files can be imported into '{}'",
                dest.raw
            )));
        }

        let target = dest.join(&name)?;
        import_local_file(vfs, &entry.path(), &target).await?;
        copied += 1;
    }
    Ok(copied)
}

async fn import_local_file(
    vfs: &mut VfsContext,
    src_path: &Path,
    dest: &IcloudSpec,
) -> icloud_api::Result<()> {
    let bytes = std::fs::read(src_path)?;
    validate_import_bytes(dest, &bytes)?;
    vfs.fs
        .write_file(&dest.normalized, &bytes)
        .await
        .map_err(map_fs_error)?;
    Ok(())
}

async fn copy_icloud_to_local(
    vfs: &mut VfsContext,
    src: &IcloudSpec,
    dest: &LocalSpec,
    recursive: bool,
) -> icloud_api::Result<usize> {
    match &src.target {
        VfsTarget::NotesFile { .. }
        | VfsTarget::RemindersFile { .. }
        | VfsTarget::HideMyEmailAliases => {
            let target_path = resolve_local_file_destination(dest, src)?;
            export_icloud_file(vfs, src, &target_path).await?;
            Ok(1)
        }
        VfsTarget::Root
        | VfsTarget::NotesRoot
        | VfsTarget::NotesFolder { .. }
        | VfsTarget::RemindersRoot
        | VfsTarget::RemindersList { .. }
        | VfsTarget::HideMyEmailRoot => {
            if !recursive {
                return Err(icloud_api::Error::Usage(
                    "copying an iCloud directory requires --recursive".into(),
                ));
            }
            export_icloud_dir(vfs, src, &dest.path).await
        }
        _ => Err(icloud_api::Error::Usage(format!(
            "cannot copy '{}' to the host filesystem",
            src.raw
        ))),
    }
}

async fn export_icloud_file(
    vfs: &mut VfsContext,
    src: &IcloudSpec,
    dest_path: &Path,
) -> icloud_api::Result<()> {
    let bytes = vfs
        .fs
        .read_file_buffer(&src.normalized)
        .await
        .map_err(map_fs_error)?;
    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest_path, bytes)?;
    Ok(())
}

async fn export_icloud_dir(
    vfs: &mut VfsContext,
    src: &IcloudSpec,
    dest_root: &Path,
) -> icloud_api::Result<usize> {
    std::fs::create_dir_all(dest_root)?;
    let mut copied = 0usize;
    let mut stack = vec![(src.normalized.clone(), dest_root.to_path_buf())];

    while let Some((src_dir, local_dir)) = stack.pop() {
        std::fs::create_dir_all(&local_dir)?;
        let entries = vfs
            .fs
            .readdir_with_file_types(&src_dir)
            .await
            .map_err(map_fs_error)?;

        for entry in entries {
            let child_src = join_vfs(&src_dir, &entry.name);
            let child_dest = local_dir.join(&entry.name);
            if entry.is_directory {
                stack.push((child_src, child_dest));
            } else {
                let bytes = vfs
                    .fs
                    .read_file_buffer(&child_src)
                    .await
                    .map_err(map_fs_error)?;
                std::fs::write(child_dest, bytes)?;
                copied += 1;
            }
        }
    }

    Ok(copied)
}

async fn ensure_import_destination(
    vfs: &mut VfsContext,
    dest: &IcloudSpec,
) -> icloud_api::Result<()> {
    let parent = match &dest.target {
        VfsTarget::NotesFolder { .. } | VfsTarget::RemindersList { .. } => dest.clone(),
        VfsTarget::NotesFile { folder_name, .. } => {
            IcloudSpec::parse(&format!("icloud:/Notes/{folder_name}"))?
        }
        VfsTarget::RemindersFile { list_name, .. } => {
            IcloudSpec::parse(&format!("icloud:/Reminders/{list_name}"))?
        }
        VfsTarget::NotesRoot | VfsTarget::RemindersRoot => return Ok(()),
        _ => {
            return Err(icloud_api::Error::Usage(format!(
                "cannot import host data into '{}'",
                dest.raw
            )))
        }
    };

    if !vfs.fs.exists(&parent.normalized).await {
        vfs.fs
            .mkdir(&parent.normalized, &MkdirOptions { recursive: false })
            .await
            .map_err(map_fs_error)?;
    }
    Ok(())
}

fn resolve_icloud_file_destination(
    dest: &IcloudSpec,
    src_path: &Path,
) -> icloud_api::Result<IcloudSpec> {
    match dest.target {
        VfsTarget::NotesFile { .. } | VfsTarget::RemindersFile { .. } => Ok(dest.clone()),
        VfsTarget::NotesFolder { .. } | VfsTarget::RemindersList { .. } => {
            let name = src_path
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or_else(|| {
                    icloud_api::Error::Usage("source file has no valid UTF-8 name".into())
                })?;
            if !name.ends_with(".md") {
                return Err(icloud_api::Error::Usage(format!(
                    "source file '{}' must end with .md when copying into '{}'",
                    src_path.display(),
                    dest.raw
                )));
            }
            dest.join(name)
        }
        VfsTarget::NotesRoot | VfsTarget::RemindersRoot => Err(icloud_api::Error::Usage(format!(
            "destination '{}' must include a folder/list or file path",
            dest.raw
        ))),
        _ => Err(icloud_api::Error::Usage(format!(
            "cannot import a host file into '{}'",
            dest.raw
        ))),
    }
}

fn resolve_local_file_destination(
    dest: &LocalSpec,
    src: &IcloudSpec,
) -> icloud_api::Result<PathBuf> {
    let treat_as_dir = dest.raw.ends_with(std::path::MAIN_SEPARATOR) || dest.path.is_dir();
    if treat_as_dir {
        let name = src.basename().ok_or_else(|| {
            icloud_api::Error::Usage(format!("cannot derive a filename from '{}'", src.raw))
        })?;
        return Ok(dest.path.join(name));
    }
    Ok(dest.path.clone())
}

fn validate_import_bytes(dest: &IcloudSpec, bytes: &[u8]) -> icloud_api::Result<()> {
    match dest.target {
        VfsTarget::NotesFile { .. } | VfsTarget::RemindersFile { .. } => {
            std::str::from_utf8(bytes).map_err(|_| {
                icloud_api::Error::Usage(format!(
                    "import into '{}' requires valid UTF-8 Markdown",
                    dest.raw
                ))
            })?;
            Ok(())
        }
        _ => Ok(()),
    }
}

fn join_vfs(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{parent}/{child}")
    }
}

fn os_to_string(name: impl AsRef<OsStr>) -> icloud_api::Result<String> {
    name.as_ref()
        .to_str()
        .map(ToString::to_string)
        .ok_or_else(|| icloud_api::Error::Usage("encountered a non-UTF-8 local path".into()))
}

fn map_fs_error(error: bashbox::fs::types::FsError) -> icloud_api::Error {
    use bashbox::fs::types::FsError;

    match error {
        FsError::NotFound { path, .. } => {
            icloud_api::Error::Usage(format!("path not found: {path}"))
        }
        FsError::AlreadyExists { path, .. } => {
            icloud_api::Error::Usage(format!("path already exists: {path}"))
        }
        FsError::IsDirectory { path, .. } => {
            icloud_api::Error::Usage(format!("expected a file, found directory: {path}"))
        }
        FsError::NotDirectory { path, .. } => {
            icloud_api::Error::Usage(format!("expected a directory: {path}"))
        }
        FsError::NotEmpty { path, .. } => {
            icloud_api::Error::Usage(format!("directory not empty: {path}"))
        }
        FsError::InvalidArgument { path, .. } => {
            icloud_api::Error::Usage(format!("invalid iCloud path or payload: {path}"))
        }
        FsError::SymlinkLoop { path, .. } => {
            icloud_api::Error::Usage(format!("symlink loop detected: {path}"))
        }
        FsError::PermissionDenied { path, .. } => {
            icloud_api::Error::Usage(format!("permission denied for iCloud path: {path}"))
        }
        FsError::ReadOnly { operation } => icloud_api::Error::Usage(format!(
            "read-only iCloud target for operation '{operation}'"
        )),
        FsError::Other { message } => {
            if message.contains("Session expired") {
                icloud_api::Error::Session(message)
            } else if message.starts_with("EXDEV:") {
                icloud_api::Error::Usage(message)
            } else {
                icloud_api::Error::Transfer(message)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SearchRoot {
    Notes,
    Reminders,
}

impl SearchRoot {
    fn collection_label(self) -> &'static str {
        match self {
            SearchRoot::Notes => "note folders",
            SearchRoot::Reminders => "reminder lists",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_icloud_spec() {
        let spec = IcloudSpec::parse("icloud:/Notes/Work/Quarterly.md").unwrap();
        assert_eq!(spec.normalized, "/Notes/Work/Quarterly.md");
    }

    #[test]
    fn resolves_dir_destination() {
        let local = LocalSpec {
            raw: "./out/".into(),
            path: PathBuf::from("./out"),
        };
        let remote = IcloudSpec::parse("icloud:/HideMyEmail/aliases.json").unwrap();
        assert_eq!(
            resolve_local_file_destination(&local, &remote).unwrap(),
            PathBuf::from("./out/aliases.json")
        );
    }
}
