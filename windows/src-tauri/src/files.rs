// Dropped files are copied into %LOCALAPPDATA%\Coucou\inbox so the original is
// never touched and the copy survives the drag source going away.
// The inbox is swept of anything older than a week, as on macOS.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::settings;

const KEEP_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DroppedFile {
    pub name: String,
    pub path: String,
    pub size: u64,
}

pub fn inbox_dir() -> PathBuf {
    settings::local_dir().join("inbox")
}

pub fn ingest(source: &str) -> Result<DroppedFile, String> {
    ingest_into(Path::new(source), &inbox_dir())
}

/// Only regular files copied into this application's inbox can be attached.
/// Canonical paths prevent ../, directory junctions and symlinks from escaping.
pub fn checked_inbox_path(path: &str) -> Result<PathBuf, String> {
    checked_path_in(Path::new(path), &inbox_dir())
}

fn checked_path_in(path: &Path, inbox: &Path) -> Result<PathBuf, String> {
    let root =
        std::fs::canonicalize(inbox).map_err(|_| "Attachment inbox unavailable.".to_string())?;
    let meta =
        std::fs::symlink_metadata(path).map_err(|_| "Attachment unavailable.".to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("Only regular inbox files can be attached.".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // All reparse points, not only symlinks (junctions/cloud placeholders).
        if meta.file_attributes() & 0x400 != 0 {
            return Err("Attachment cannot be a reparse point.".into());
        }
    }
    if path
        .file_name()
        .map(|name| name.to_string_lossy().contains(':'))
        .unwrap_or(true)
    {
        return Err("Attachment cannot reference an alternate data stream.".into());
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|_| "Attachment unavailable.".to_string())?;
    // Ingest creates direct children only; do not accept arbitrary subtrees.
    if canonical.parent() != Some(root.as_path()) {
        return Err("Attachments must be copied into the inbox first.".into());
    }
    Ok(canonical)
}

fn ingest_into(src: &Path, dir: &Path) -> Result<DroppedFile, String> {
    // Refuse devices and named pipes before opening: opening one may block.
    if !std::fs::metadata(src)
        .map_err(|e| format!("Cannot read attachment: {e}"))?
        .is_file()
    {
        return Err("Only regular files can be dropped.".into());
    }
    let mut input = std::fs::File::open(src).map_err(|e| format!("Cannot read attachment: {e}"))?;
    let meta = input
        .metadata()
        .map_err(|e| format!("Cannot read attachment: {e}"))?;
    if !meta.is_file() {
        return Err("Only regular files can be dropped.".into());
    }

    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());

    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = src
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();
    let mut reserved = None;
    for i in 1..1000 {
        let candidate = if i == 1 {
            dir.join(&name)
        } else {
            dir.join(format!("{stem} ({i}){ext}"))
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(output) => {
                reserved = Some((candidate, output));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Cannot copy attachment: {error}")),
        }
    }
    let (dest, mut output) =
        reserved.ok_or_else(|| "Too many attachments with this name.".to_string())?;
    let copied = match std::io::copy(&mut input, &mut output)
        .and_then(|size| output.sync_all().map(|_| size))
    {
        Ok(size) => size,
        Err(error) => {
            drop(output);
            let _ = std::fs::remove_file(&dest);
            return Err(format!("Cannot copy attachment: {error}"));
        }
    };
    // The copy's retention window starts now, not at the source's old timestamp.
    let _ = output.set_modified(SystemTime::now());
    drop(output);
    if let Err(error) = checked_path_in(&dest, dir) {
        let _ = std::fs::remove_file(&dest);
        return Err(error);
    }
    sweep(dir);

    Ok(DroppedFile {
        name,
        path: dest.to_string_lossy().to_string(),
        size: copied,
    })
}

/// Drops anything copied here more than a week ago. `ingest` stamps every copy
/// with the time it landed, so this really is the age of the copy and not the
/// age of whatever the user happened to drag in.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !meta.is_file() || meta.file_type().is_symlink() {
            continue;
        }
        let Ok(copied) = meta.modified() else {
            continue;
        };
        if now
            .duration_since(copied)
            .map(|age| age > KEEP_FOR)
            .unwrap_or(false)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_copies_and_never_overwrites() {
        let tmp = temp_root("copies");
        std::fs::create_dir_all(&tmp).unwrap();
        let inbox = tmp.join("inbox");
        let source = tmp.join("note.txt");
        std::fs::write(&source, b"hello").unwrap();

        let first = ingest_into(&source, &inbox).unwrap();
        assert_eq!(first.name, "note.txt");
        assert_eq!(std::fs::read(&first.path).unwrap(), b"hello");

        // A second drop of the same name must not clobber the first copy.
        std::fs::write(&source, b"second").unwrap();
        let second = ingest_into(&source, &inbox).unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(std::fs::read(&first.path).unwrap(), b"hello");
        assert_eq!(std::fs::read(&second.path).unwrap(), b"second");

        // Folders are refused rather than silently ignored.
        assert!(ingest_into(&tmp, &inbox).is_err());

        // An ancient source must not arrive already older than the sweep window.
        let old_source = tmp.join("ancient.txt");
        std::fs::write(&old_source, b"old").unwrap();
        let long_ago = SystemTime::now() - KEEP_FOR - Duration::from_secs(60 * 60);
        std::fs::File::options()
            .write(true)
            .open(&old_source)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
        let aged = ingest_into(&old_source, &inbox).unwrap();
        assert!(
            Path::new(&aged.path).exists(),
            "a file copied just now was swept as if it were a week old"
        );
        let _ = std::fs::remove_file(&aged.path);

        let _ = std::fs::remove_file(&first.path);
        let _ = std::fs::remove_file(&second.path);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn temp_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "coucou-files-{label}-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn confinement_refuses_external_files_traversal_directories_and_missing_files() {
        let tmp = temp_root("confinement");
        let inbox = tmp.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let inside = inbox.join("note.txt");
        let outside = tmp.join("secret.txt");
        std::fs::write(&inside, b"public").unwrap();
        std::fs::write(&outside, b"private").unwrap();
        assert_eq!(
            checked_path_in(&inside, &inbox).unwrap(),
            std::fs::canonicalize(&inside).unwrap()
        );
        assert!(checked_path_in(&outside, &inbox).is_err());
        assert!(checked_path_in(&inbox.join("../secret.txt"), &inbox).is_err());
        assert!(checked_path_in(&inbox, &inbox).is_err());
        assert!(checked_path_in(&inbox.join("missing.txt"), &inbox).is_err());
        let nested = inbox.join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("secret.txt"), b"nested").unwrap();
        assert!(checked_path_in(&nested.join("secret.txt"), &inbox).is_err());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn destination_name_exhaustion_refuses_without_overwriting() {
        let tmp = temp_root("collision");
        let inbox = tmp.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let source = tmp.join("note.txt");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(inbox.join("note.txt"), b"keep").unwrap();
        for i in 2..1000 {
            std::fs::write(inbox.join(format!("note ({i}).txt")), b"keep").unwrap();
        }
        assert!(ingest_into(&source, &inbox).is_err());
        assert_eq!(std::fs::read(inbox.join("note.txt")).unwrap(), b"keep");
        assert_eq!(
            std::fs::read(inbox.join("note (999).txt")).unwrap(),
            b"keep"
        );
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn alternate_data_stream_cannot_be_attached() {
        let tmp = temp_root("ads");
        let inbox = tmp.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let path = inbox.join("note.txt");
        std::fs::write(&path, b"public").unwrap();
        let stream = PathBuf::from(format!("{}:hidden", path.display()));
        std::fs::write(&stream, b"secret").unwrap();
        assert!(checked_path_in(&stream, &inbox).is_err());
        assert!(checked_path_in(&path, &inbox).is_ok());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_inside_inbox_cannot_attach_external_content() {
        let tmp = temp_root("symlink");
        let inbox = tmp.join("inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let outside = tmp.join("secret.txt");
        std::fs::write(&outside, b"private").unwrap();
        let link = inbox.join("link.txt");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(checked_path_in(&link, &inbox).is_err());
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
