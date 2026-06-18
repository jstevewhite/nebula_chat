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

/// Accepts a YAML scalar string OR a sequence of strings (Claude's
/// `allowed-tools` appears in both shapes across skills).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum StringOrVec {
    One(String),
    Many(Vec<String>),
}

fn normalize_tools(v: Option<StringOrVec>) -> Vec<String> {
    match v {
        None => Vec::new(),
        Some(StringOrVec::One(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        Some(StringOrVec::Many(xs)) => xs
            .into_iter()
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ClaudeFrontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "allowed-tools")]
    allowed_tools: Option<StringOrVec>,
}

/// Parse one `<dir>/SKILL.md` into a `Skill` (origin = Claude) plus its
/// normalized `allowed-tools` list. `slug` must be a valid slug and is the
/// directory name as it appears in the scanned dir (the symlink/entry name),
/// not derived from `skill_dir` (which may be a canonicalized symlink target).
pub fn read_claude_skill(skill_dir: &Path, slug: &str) -> Result<(Skill, Vec<String>)> {
    if !super::is_valid_slug(slug) {
        return Err(anyhow!("invalid slug '{slug}'"));
    }

    let md_path = skill_dir.join("SKILL.md");
    let raw = fs::read_to_string(&md_path).with_context(|| format!("read {}", md_path.display()))?;
    let (front_str, body) = split_frontmatter(&raw);
    let front: ClaudeFrontmatter = if front_str.trim().is_empty() {
        ClaudeFrontmatter::default()
    } else {
        serde_yaml::from_str(front_str)
            .with_context(|| format!("parse frontmatter in {}", md_path.display()))?
    };

    if front.description.trim().is_empty() {
        return Err(anyhow!("claude skill {slug} is missing a description"));
    }
    let name = if front.name.is_empty() {
        slug.to_string()
    } else {
        front.name.clone()
    };
    let allowed_tools = normalize_tools(front.allowed_tools);

    let skill = Skill {
        slug: slug.to_string(),
        name,
        description: front.description,
        body: body.trim_start_matches('\n').to_string(),
        built_in: false,
        path: md_path,
        origin: super::api::SkillOrigin::Claude,
    };
    Ok((skill, allowed_tools))
}

/// Scan `dir` (e.g. `~/.claude/skills/`) for `<name>/SKILL.md` skills. Follows
/// symlinks via canonicalize. Unparseable entries are logged and skipped so a
/// single bad skill can't break discovery. Returns each skill with its
/// `allowed-tools` list.
pub fn scan_claude_skills(dir: &Path) -> Vec<(Skill, Vec<String>)> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out, // missing/unreadable dir → empty, non-fatal
    };
    for entry in entries.flatten() {
        let slug = entry.file_name().to_string_lossy().into_owned();
        // Canonicalize to resolve symlinks (most of ~/.claude/skills are links).
        let path = match fs::canonicalize(entry.path()) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("skip claude skill {}: {e}", entry.path().display());
                continue;
            }
        };
        if !path.is_dir() || !path.join("SKILL.md").is_file() {
            continue;
        }
        match read_claude_skill(&path, &slug) {
            Ok(found) => out.push(found),
            Err(e) => tracing::warn!("skip claude skill {}: {e}", path.display()),
        }
    }
    out
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

/// Best-effort classification of a Claude skill as a self-sufficient
/// instruction skill (default ON) vs a script/subagent launcher Nebula can't
/// run (default OFF). Deliberately conservative toward inclusion: only a strong
/// "this is a doer" signal flips it off. The Settings checklist is the user's
/// authoritative override.
pub fn claude_skill_default_enabled(body: &str, allowed_tools: &[String]) -> bool {
    const EXEC_TOOLS: [&str; 5] = ["bash", "write", "edit", "shell", "execute"];
    if allowed_tools.iter().any(|t| {
        let t = t.to_ascii_lowercase();
        EXEC_TOOLS.iter().any(|e| t.contains(e))
    }) {
        return false;
    }

    let lower = body.to_lowercase();

    const SUBAGENT_MARKERS: [&str; 4] =
        ["dispatch a subagent", "spawn ", "task tool", "subagent"];
    if SUBAGENT_MARKERS.iter().any(|m| lower.contains(m)) {
        return false;
    }

    // Script-execution imperative = a run verb AND a script extension present.
    const RUN_VERBS: [&str; 4] = ["run ", "python ", "bash ", "./"];
    const SCRIPT_EXT: [&str; 3] = [".py", ".sh", ".js"];
    let mentions_script = SCRIPT_EXT.iter().any(|x| lower.contains(x));
    let has_run_verb = RUN_VERBS.iter().any(|v| lower.contains(v));
    if mentions_script && has_run_verb {
        return false;
    }

    true
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

    fn write_claude_skill(root: &Path, name: &str, frontmatter: &str, body: &str) {
        let d = root.join(name);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("SKILL.md"), format!("---\n{frontmatter}\n---\n{body}\n")).unwrap();
    }

    #[test]
    fn scan_claude_skills_reads_dir_based_skill() {
        let root = TempDir::new().unwrap();
        write_claude_skill(root.path(), "brainstorming", "name: Brainstorming\ndescription: explore ideas", "Do the thing.");
        let found = scan_claude_skills(root.path());
        assert_eq!(found.len(), 1);
        let (skill, tools) = &found[0];
        assert_eq!(skill.slug, "brainstorming");
        assert_eq!(skill.name, "Brainstorming");
        assert_eq!(skill.description, "explore ideas");
        assert!(skill.body.contains("Do the thing"));
        assert_eq!(skill.origin, crate::skills::SkillOrigin::Claude);
        assert!(!skill.built_in);
        assert!(tools.is_empty());
    }

    #[test]
    fn scan_claude_skills_skips_dirs_without_skill_md() {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("empty")).unwrap();
        write_claude_skill(root.path(), "ok", "description: d", "b");
        let found = scan_claude_skills(root.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.slug, "ok");
    }

    #[test]
    fn scan_claude_skills_skips_missing_description() {
        let root = TempDir::new().unwrap();
        write_claude_skill(root.path(), "nodesc", "name: No Desc", "b");
        assert!(scan_claude_skills(root.path()).is_empty());
    }

    #[test]
    fn scan_claude_skills_parses_allowed_tools_list_and_csv() {
        let root = TempDir::new().unwrap();
        write_claude_skill(root.path(), "listy", "description: d\nallowed-tools:\n  - Read\n  - Bash", "b");
        write_claude_skill(root.path(), "csvy", "description: d\nallowed-tools: Read, Bash", "b");
        let mut found = scan_claude_skills(root.path());
        found.sort_by(|a, b| a.0.slug.cmp(&b.0.slug));
        assert_eq!(found[0].1, vec!["Read".to_string(), "Bash".to_string()]); // csvy
        assert_eq!(found[1].1, vec!["Read".to_string(), "Bash".to_string()]); // listy
    }

    #[test]
    fn scan_claude_skills_missing_dir_is_empty() {
        let root = TempDir::new().unwrap();
        let missing = root.path().join("nope");
        assert!(scan_claude_skills(&missing).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn scan_claude_skills_slug_from_symlink_name_not_target() {
        // Target dir has a different basename than the symlink that points to it.
        let target_root = TempDir::new().unwrap();
        write_claude_skill(target_root.path(), "target-basename", "description: d", "b");
        let scan_root = TempDir::new().unwrap();
        std::os::unix::fs::symlink(
            target_root.path().join("target-basename"),
            scan_root.path().join("nice-slug"),
        )
        .unwrap();
        let found = scan_claude_skills(scan_root.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0.slug, "nice-slug");
    }

    #[test]
    fn heuristic_instruction_skill_defaults_on() {
        let body = "Help the user explore ideas one question at a time. Present a design and get approval.";
        assert!(claude_skill_default_enabled(body, &[]));
    }

    #[test]
    fn heuristic_exec_allowed_tools_defaults_off() {
        assert!(!claude_skill_default_enabled("guidance", &["Read".into(), "Bash".into()]));
        assert!(!claude_skill_default_enabled("guidance", &["Write".into()]));
        assert!(!claude_skill_default_enabled("guidance", &["Edit".into()]));
    }

    #[test]
    fn heuristic_script_launcher_defaults_off() {
        let body = "Run the sanitize.py script: `python sanitize.py input.txt`.";
        assert!(!claude_skill_default_enabled(body, &[]));
    }

    #[test]
    fn heuristic_subagent_skill_defaults_off() {
        let body = "First, dispatch a subagent to gather context, then synthesize.";
        assert!(!claude_skill_default_enabled(body, &[]));
    }

    #[test]
    fn heuristic_bare_scripts_dir_mention_stays_on() {
        // Mentioning a .md companion or having a scripts/ dir is NOT a run
        // imperative — must stay ON (this is the `brainstorming` case).
        let body = "If they accept, read visual-companion.md for the detailed guide.";
        assert!(claude_skill_default_enabled(body, &[]));
    }
}
