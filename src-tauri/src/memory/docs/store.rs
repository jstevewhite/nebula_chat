//! On-disk markdown store with YAML frontmatter. Files live at
//! `<docs_dir>/<slug>.md`. Atomic writes via temp-file + rename.

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::links::is_valid_slug;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Frontmatter {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct ParsedDoc {
    pub front: Frontmatter,
    pub body: String,
    pub path: PathBuf,
    pub mtime_ns: i64,
}

/// Resolve the path on disk for a given slug, without checking existence.
pub fn path_for(docs_dir: &Path, id: &str) -> PathBuf {
    docs_dir.join(format!("{}.md", id))
}

/// Read and parse a doc by absolute path. Returns the parsed frontmatter,
/// body, and mtime_ns. If the file has no frontmatter, the title defaults
/// to the filename stem and id is derived from it (the caller should treat
/// such files as best-effort imports).
pub fn read_path(path: &Path) -> Result<ParsedDoc> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read doc {}", path.display()))?;
    let (front, body) = split_frontmatter(&raw);
    let mut front: Frontmatter = if front.trim().is_empty() {
        Frontmatter::default()
    } else {
        serde_yaml::from_str(front)
            .with_context(|| format!("parse frontmatter in {}", path.display()))?
    };

    if front.id.is_empty() {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            front.id = stem.to_string();
        }
    }
    if front.title.is_empty() {
        front.title = front.id.clone();
    }

    let mtime_ns = mtime_ns(path)?;
    Ok(ParsedDoc {
        front,
        body: body.to_string(),
        path: path.to_path_buf(),
        mtime_ns,
    })
}

/// Write a doc atomically. `created_at` is preserved from the existing file's
/// frontmatter when present; `updated_at` is always refreshed.
pub fn write_doc(
    docs_dir: &Path,
    id: &str,
    title: &str,
    tags: &[String],
    links: &[String],
    body: &str,
    preserve_created_at: Option<&str>,
) -> Result<(PathBuf, String, String)> {
    if !is_valid_slug(id) {
        return Err(anyhow!(
            "invalid doc id '{id}': must match [a-z0-9][a-z0-9-]{{0,63}}"
        ));
    }
    fs::create_dir_all(docs_dir)?;
    let path = path_for(docs_dir, id);

    let now = Utc::now().to_rfc3339();
    let created_at = preserve_created_at
        .map(|s| s.to_string())
        .unwrap_or_else(|| now.clone());

    let front = Frontmatter {
        id: id.to_string(),
        title: title.to_string(),
        tags: tags.to_vec(),
        links: links.to_vec(),
        created_at: created_at.clone(),
        updated_at: now.clone(),
    };

    let yaml = serde_yaml::to_string(&front).context("serialize frontmatter")?;
    let mut out = String::with_capacity(yaml.len() + body.len() + 16);
    out.push_str("---\n");
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }

    atomic_write(&path, out.as_bytes())?;
    Ok((path, created_at, now))
}

/// Delete a doc file. Returns Ok if the file did not exist.
pub fn delete_doc(docs_dir: &Path, id: &str) -> Result<()> {
    let p = path_for(docs_dir, id);
    if p.exists() {
        fs::remove_file(&p).with_context(|| format!("delete {}", p.display()))?;
    }
    Ok(())
}

/// Scan the docs dir for `*.md` files and return their absolute paths.
pub fn scan_dir(docs_dir: &Path) -> Result<Vec<PathBuf>> {
    if !docs_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(docs_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
            out.push(path);
        }
    }
    Ok(out)
}

/// Read the body of a doc by id, with no frontmatter, for `memory_doc_fetch`.
pub fn read_id(docs_dir: &Path, id: &str) -> Result<Option<ParsedDoc>> {
    let p = path_for(docs_dir, id);
    if !p.exists() {
        return Ok(None);
    }
    Ok(Some(read_path(&p)?))
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("no parent dir for {}", target.display()))?;
    fs::create_dir_all(parent)?;
    let mut tmp = parent.join(format!(
        ".{}.tmp.{}",
        target.file_name().and_then(|s| s.to_str()).unwrap_or("doc"),
        std::process::id()
    ));
    {
        let mut f = fs::File::create(&tmp)
            .with_context(|| format!("create temp {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    // Best-effort: replace target.
    if let Err(e) = fs::rename(&tmp, target) {
        // On Windows the rename may fail if target exists. Fall back to remove+rename.
        let _ = fs::remove_file(target);
        if let Err(e2) = fs::rename(&tmp, target) {
            tmp = PathBuf::new(); // suppress cleanup of moved file
            let _ = tmp;
            return Err(anyhow!("atomic rename failed: {e} / fallback: {e2}"));
        }
    }
    Ok(())
}

fn split_frontmatter(raw: &str) -> (&str, &str) {
    // Recognise leading `---\n...\n---\n` blocks.
    let bytes = raw.as_bytes();
    if !raw.starts_with("---") {
        return ("", raw);
    }
    let after_first = match raw.find('\n') {
        Some(p) => p + 1,
        None => return ("", raw),
    };
    // Find the closing `---` at the start of a line.
    let rest = &raw[after_first..];
    if let Some(idx) = find_closing(rest) {
        let front = &rest[..idx];
        // Skip past the closing fence line.
        let after_close = &rest[idx..];
        let nl = match after_close.find('\n') {
            Some(p) => p + 1,
            None => after_close.len(),
        };
        let body = &after_close[nl..];
        (front, body)
    } else {
        // Malformed; treat whole thing as body.
        let _ = bytes;
        ("", raw)
    }
}

fn find_closing(rest: &str) -> Option<usize> {
    let mut search_from = 0;
    while search_from < rest.len() {
        let line_end = rest[search_from..]
            .find('\n')
            .map(|p| search_from + p)
            .unwrap_or(rest.len());
        let line = &rest[search_from..line_end];
        if line.trim_end() == "---" {
            return Some(search_from);
        }
        if line_end >= rest.len() {
            return None;
        }
        search_from = line_end + 1;
    }
    None
}

fn mtime_ns(path: &Path) -> Result<i64> {
    let meta = fs::metadata(path)?;
    let mt = meta.modified()?;
    let dur = mt
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| anyhow!("mtime before epoch: {e}"))?;
    let secs = dur.as_secs() as i64;
    let nanos = dur.subsec_nanos() as i64;
    Ok(secs.saturating_mul(1_000_000_000).saturating_add(nanos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn write_and_read_roundtrip() {
        let dir = TempDir::new().unwrap();
        let docs = dir.path().to_path_buf();

        let (path, created, updated) = write_doc(
            &docs,
            "user-profile",
            "User Profile",
            &["profile".into()],
            &["project-nebula".into()],
            "Hello world.\n",
            None,
        )
        .unwrap();

        assert!(path.exists());
        assert_eq!(created, updated, "created==updated on first write");

        let parsed = read_path(&path).unwrap();
        assert_eq!(parsed.front.id, "user-profile");
        assert_eq!(parsed.front.title, "User Profile");
        assert_eq!(parsed.front.tags, vec!["profile".to_string()]);
        assert_eq!(parsed.front.links, vec!["project-nebula".to_string()]);
        assert!(parsed.body.contains("Hello world"));
    }

    #[test]
    fn second_write_preserves_created_at() {
        let dir = TempDir::new().unwrap();
        let docs = dir.path().to_path_buf();
        let (_p, created1, _u1) =
            write_doc(&docs, "p", "Title", &[], &[], "v1", None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let (_p, created2, updated2) =
            write_doc(&docs, "p", "Title2", &[], &[], "v2", Some(&created1))
                .unwrap();
        assert_eq!(created1, created2);
        assert_ne!(created1, updated2);
    }

    #[test]
    fn rejects_invalid_slug() {
        let dir = TempDir::new().unwrap();
        let err = write_doc(dir.path(), "Bad Name", "T", &[], &[], "x", None);
        assert!(err.is_err());
    }

    #[test]
    fn scan_returns_only_md_files() {
        let dir = TempDir::new().unwrap();
        write_doc(dir.path(), "a", "A", &[], &[], "alpha", None).unwrap();
        fs::write(dir.path().join("notes.txt"), "ignore me").unwrap();
        let files = scan_dir(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("a.md"));
    }
}
