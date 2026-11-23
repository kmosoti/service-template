//! Epistemic knowledge graph validator (`knowledge/{decisions,research,observations}`).

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub fn check_knowledge(root: &Path) -> Result<String, String> {
    let readme_path = root.join("knowledge/README.md");
    let readme = fs::read_to_string(&readme_path)
        .map_err(|e| format!("missing knowledge/README.md: {e}"))?;

    let categories = [
        ("decisions", "D-", "decision"),
        ("research", "R-", "research"),
        ("observations", "O-", "observation"),
    ];
    let mut note_count = 0usize;

    for (dir_name, id_prefix, expected_kind) in categories {
        let dir = root.join("knowledge").join(dir_name);
        let entries = fs::read_dir(&dir).map_err(|e| format!("missing {}: {e}", dir.display()))?;
        let mut dir_notes = 0usize;

        for entry in entries {
            let entry = entry.map_err(|e| format!("read_dir error: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            dir_notes += 1;
            note_count += 1;

            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let file_name = path.file_name().unwrap().to_string_lossy();
            if !readme.contains(file_name.as_ref()) {
                return Err(format!("knowledge/README.md does not index `{rel}`"));
            }

            let content =
                fs::read_to_string(&path).map_err(|e| format!("failed reading {rel}: {e}"))?;
            let (fields, links) = parse_frontmatter(&content, &rel)?;

            let id = fields
                .get("id")
                .ok_or_else(|| format!("{rel}: missing `id` in frontmatter"))?;
            if !id.starts_with(id_prefix) {
                return Err(format!("{rel}: id `{id}` must start with `{id_prefix}`"));
            }
            let kind = fields
                .get("kind")
                .ok_or_else(|| format!("{rel}: missing `kind` in frontmatter"))?;
            if kind != expected_kind {
                return Err(format!("{rel}: kind `{kind}` must equal `{expected_kind}`"));
            }
            for req in ["status", "date", "added_by", "human_ratified"] {
                if !fields.contains_key(req) {
                    return Err(format!("{rel}: missing `{req}` in frontmatter"));
                }
            }
            let ratified = fields["human_ratified"].as_str();
            if !matches!(ratified, "true" | "false") {
                return Err(format!(
                    "{rel}: `human_ratified` must be boolean true or false, found `{ratified}`"
                ));
            }
            if links.is_empty() {
                return Err(format!(
                    "{rel}: `links` must contain at least one graph edge"
                ));
            }
            for link in links {
                let target = root.join(&link);
                if !target.is_file() {
                    return Err(format!("{rel}: broken knowledge link `{link}`"));
                }
            }
        }
        if dir_notes == 0 {
            return Err(format!(
                "knowledge/{dir_name} must contain at least one note"
            ));
        }
    }

    Ok(format!("{note_count} linked epistemic notes validated"))
}

fn parse_frontmatter(
    content: &str,
    rel: &str,
) -> Result<(BTreeMap<String, String>, Vec<String>), String> {
    let rest = content
        .strip_prefix("---\n")
        .ok_or_else(|| format!("{rel}: missing opening `---` YAML frontmatter"))?;
    let end = rest
        .find("\n---\n")
        .ok_or_else(|| format!("{rel}: missing closing `---` YAML frontmatter"))?;
    let block = &rest[..end];

    let mut fields = BTreeMap::new();
    let mut links = Vec::new();
    let mut in_links = false;

    for raw in block.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(item) = line.strip_prefix("- ") {
            if in_links {
                links.push(item.trim().trim_matches('"').trim_matches('\'').to_owned());
                continue;
            }
        } else {
            in_links = false;
        }

        let (k, v) = line
            .split_once(':')
            .ok_or_else(|| format!("{rel}: malformed frontmatter line `{line}`"))?;
        let key = k.trim().to_owned();
        let val = v.trim();
        if key == "links" {
            if val.starts_with('[') && val.ends_with(']') {
                for part in val[1..val.len() - 1].split(',') {
                    let clean = part.trim().trim_matches('"').trim_matches('\'');
                    if !clean.is_empty() {
                        links.push(clean.to_owned());
                    }
                }
            } else {
                in_links = true;
            }
            fields.insert(key, String::new());
        } else {
            fields.insert(key, val.trim_matches('"').trim_matches('\'').to_owned());
        }
    }
    Ok((fields, links))
}
