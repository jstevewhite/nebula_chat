//! Disk I/O + YAML-frontmatter parsing for skill markdown files.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use super::api::Skill;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Frontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    built_in: bool,
}

/// Scan the user skills dir + the `built-ins/` subdir and return all parseable
/// skills. Files that fail to parse are logged and skipped so a malformed file
/// can't break the chat.
pub fn scan_all(skills_dir: &Path) -> Result<Vec<Skill>> {
    let mut out = Vec::new();
    if !skills_dir.exists() {
        return Ok(out);
    }
    push_md_files(&mut out, skills_dir, false)?;
    let built_ins = skills_dir.join("built-ins");
    if built_ins.exists() {
        push_md_files(&mut out, &built_ins, true)?;
    }
    Ok(out)
}

fn push_md_files(out: &mut Vec<Skill>, dir: &Path, built_in_flag: bool) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        match read_skill(&path, built_in_flag) {
            Ok(skill) => out.push(skill),
            Err(e) => tracing::warn!("skip unparseable skill {}: {e}", path.display()),
        }
    }
    Ok(())
}

pub fn read_skill(path: &Path, built_in_flag: bool) -> Result<Skill> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let (front_str, body) = split_frontmatter(&raw);
    let front: Frontmatter = if front_str.trim().is_empty() {
        Frontmatter::default()
    } else {
        serde_yaml::from_str(front_str)
            .with_context(|| format!("parse frontmatter in {}", path.display()))?
    };

    let slug = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("non-utf8 filename {}", path.display()))?
        .to_string();

    let name = if front.name.is_empty() {
        slug.clone()
    } else {
        front.name
    };
    if front.description.trim().is_empty() {
        return Err(anyhow!("skill {slug} is missing a description"));
    }

    Ok(Skill {
        slug,
        name,
        description: front.description,
        body: body.trim_start_matches('\n').to_string(),
        built_in: built_in_flag || front.built_in,
        path: path.to_path_buf(),
        origin: super::api::SkillOrigin::Native,
    })
}

pub fn write_skill(
    path: &PathBuf,
    slug: &str,
    name: &str,
    description: &str,
    body: &str,
    built_in_flag: bool,
) -> Result<()> {
    let front = Frontmatter {
        name: name.to_string(),
        description: description.to_string(),
        built_in: built_in_flag,
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

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, out).with_context(|| format!("write {}", path.display()))?;
    let _ = slug;
    Ok(())
}

fn split_frontmatter(raw: &str) -> (&str, &str) {
    if !raw.starts_with("---") {
        return ("", raw);
    }
    let after_first = match raw.find('\n') {
        Some(p) => p + 1,
        None => return ("", raw),
    };
    let rest = &raw[after_first..];
    let mut from = 0usize;
    while from < rest.len() {
        let line_end = rest[from..]
            .find('\n')
            .map(|p| from + p)
            .unwrap_or(rest.len());
        let line = &rest[from..line_end];
        if line.trim_end() == "---" {
            let after_close = &rest[line_end..];
            let nl = after_close.find('\n').map(|p| p + 1).unwrap_or(after_close.len());
            return (&rest[..from], &after_close[nl..]);
        }
        if line_end >= rest.len() {
            break;
        }
        from = line_end + 1;
    }
    ("", raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn read_skill_parses_frontmatter_and_body() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("code-review.md");
        fs::write(
            &path,
            "---\nname: Code Review\ndescription: when reviewing code\n---\nBody here.\n",
        )
        .unwrap();

        let s = read_skill(&path, false).unwrap();
        assert_eq!(s.slug, "code-review");
        assert_eq!(s.name, "Code Review");
        assert_eq!(s.description, "when reviewing code");
        assert!(s.body.contains("Body here"));
        assert!(!s.built_in);
    }

    #[test]
    fn read_skill_uses_slug_as_name_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("plain.md");
        fs::write(&path, "---\ndescription: a plain one\n---\nhi\n").unwrap();
        let s = read_skill(&path, false).unwrap();
        assert_eq!(s.name, "plain");
    }

    #[test]
    fn read_skill_rejects_missing_description() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.md");
        fs::write(&path, "---\nname: Nope\n---\nbody\n").unwrap();
        assert!(read_skill(&path, false).is_err());
    }

    #[test]
    fn write_then_read_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rt.md");
        write_skill(&path, "rt", "Round Trip", "describe it", "the body", false).unwrap();
        let s = read_skill(&path, false).unwrap();
        assert_eq!(s.name, "Round Trip");
        assert_eq!(s.description, "describe it");
        assert!(s.body.contains("the body"));
    }

    #[test]
    fn scan_all_picks_up_user_and_builtins() {
        let dir = TempDir::new().unwrap();
        let user = dir.path();
        fs::create_dir_all(user.join("built-ins")).unwrap();
        write_skill(
            &user.join("one.md"),
            "one",
            "One",
            "user skill",
            "body",
            false,
        )
        .unwrap();
        write_skill(
            &user.join("built-ins").join("two.md"),
            "two",
            "Two",
            "builtin",
            "body",
            true,
        )
        .unwrap();
        let all = scan_all(user).unwrap();
        assert_eq!(all.len(), 2);
        let two = all.iter().find(|s| s.slug == "two").unwrap();
        assert!(two.built_in);
    }

    #[test]
    fn read_skill_defaults_origin_to_native() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("x.md");
        fs::write(&path, "---\ndescription: d\n---\nbody\n").unwrap();
        let s = read_skill(&path, false).unwrap();
        assert_eq!(s.origin, crate::skills::SkillOrigin::Native);
    }
}
