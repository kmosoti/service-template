//! Repository hygiene checks: GitHub Action SHA pins, `.rs` line caps, and structural privacy scan.

use std::fs;
use std::path::Path;

use crate::util::walk_repo_files;

const MAX_RS_LINES: usize = 350;

pub fn check_workflow_pins(root: &Path) -> Result<String, String> {
    let wf_dir = root.join(".github/workflows");
    let entries = fs::read_dir(&wf_dir).map_err(|e| format!("missing .github/workflows: {e}"))?;
    let mut wf_count = 0usize;
    let mut pin_count = 0usize;

    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        wf_count += 1;
        let rel = path.strip_prefix(root).unwrap_or(&path).display();
        let text = fs::read_to_string(&path).map_err(|e| format!("read {rel}: {e}"))?;
        pin_count += validate_workflow_uses(&text, &rel.to_string())?;
    }
    if wf_count < 5 {
        return Err(format!("expected >= 5 workflow files, found {wf_count}"));
    }
    Ok(format!(
        "{pin_count} SHA-pinned action uses across {wf_count} workflows"
    ))
}

pub fn validate_workflow_uses(yaml: &str, label: &str) -> Result<usize, String> {
    let mut count = 0usize;
    for (idx, raw) in yaml.lines().enumerate() {
        let code = raw.split('#').next().unwrap_or("").trim();
        let Some(rest) = code
            .strip_prefix("uses:")
            .or_else(|| code.strip_prefix("- uses:"))
        else {
            continue;
        };
        let target = rest.trim().trim_matches('"').trim_matches('\'');
        if target.starts_with("./") {
            count += 1;
            continue;
        }
        let Some((repo, rev)) = target.split_once('@') else {
            return Err(format!(
                "{label}:{}: `uses: {target}` missing @<sha>",
                idx + 1
            ));
        };
        let valid_sha = rev.len() == 40
            && rev
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if repo.split('/').count() < 2 || !valid_sha {
            return Err(format!(
                "{label}:{}: unpinned action `{target}` (must pin 40-hex commit SHA)",
                idx + 1
            ));
        }
        count += 1;
    }
    Ok(count)
}

pub fn check_line_caps(root: &Path) -> Result<String, String> {
    let files = walk_repo_files(root)?;
    let mut rs_count = 0usize;
    let mut max_lines = 0usize;
    let mut max_file = String::new();

    for path in files {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        rs_count += 1;
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let text = fs::read_to_string(&path).map_err(|e| format!("read {rel}: {e}"))?;
        let lines = text.lines().count();
        if lines > max_lines {
            max_lines = lines;
            max_file = rel.clone();
        }
        if lines > MAX_RS_LINES {
            return Err(format!(
                "`{rel}` has {lines} lines, exceeding hard cap of {MAX_RS_LINES}"
            ));
        }
    }
    Ok(format!(
        "{rs_count} .rs files checked; max = {max_lines} lines ({max_file})"
    ))
}

pub fn check_privacy(root: &Path) -> Result<String, String> {
    let files = walk_repo_files(root)?;
    for path in &files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned();
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        scan_text_privacy(&text, &rel)?;
    }
    Ok(format!("{} tracked/workspace files clean", files.len()))
}

pub fn scan_text_privacy(text: &str, label: &str) -> Result<(), String> {
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        if has_host_user_path(line) {
            return Err(format!(
                "{label}:{line_no}: workstation user-directory path detected"
            ));
        }
        if has_pem_private_key(line) {
            return Err(format!(
                "{label}:{line_no}: PEM private key header detected"
            ));
        }
        if has_credential_token(line) {
            return Err(format!(
                "{label}:{line_no}: raw credential token pattern detected"
            ));
        }
    }
    Ok(())
}

fn has_host_user_path(line: &str) -> bool {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'/' {
            let rest = &line[i + 1..];
            for root_dir in ["home/", "Users/"] {
                if let Some(after_root) = rest.strip_prefix(root_dir)
                    && has_user_segment(after_root, '/')
                {
                    return true;
                }
            }
        } else if (bytes[i] == b'C' || bytes[i] == b'c')
            && line[i + 1..].starts_with(":\\Users\\")
            && has_user_segment(&line[i + 8..], '\\')
        {
            return true;
        }
    }
    false
}

fn has_user_segment(s: &str, sep: char) -> bool {
    let Some((seg, _)) = s.split_once(sep) else {
        return false;
    };
    !seg.is_empty()
        && seg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn has_pem_private_key(line: &str) -> bool {
    let Some((_, after_begin)) = line.split_once("-----BEGIN ") else {
        return false;
    };
    let Some((label, _)) = after_begin.split_once("PRIVATE KEY-----") else {
        return false;
    };
    label.len() <= 24
        && label
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b == b' ' || b == b'-')
}

fn has_credential_token(line: &str) -> bool {
    for raw in line.split(|c: char| {
        c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '=' | ':')
    }) {
        if (raw.starts_with("ghp_")
            || raw.starts_with("gho_")
            || raw.starts_with("ghs_")
            || raw.starts_with("ghu_"))
            && raw.len() >= 24
            && raw[4..].bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return true;
        }
        if let Some(rest) = raw.strip_prefix("github_pat_")
            && rest.len() >= 24
            && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return true;
        }
        if let Some(rest) = raw.strip_prefix("AKIA")
            && rest.len() == 16
            && rest
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return true;
        }
        if let Some(rest) = raw.strip_prefix("ya29.")
            && rest.len() >= 16
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        {
            return true;
        }
        if let Some(rest) = raw.strip_prefix("xox")
            && rest.len() >= 14
            && matches!(rest.as_bytes()[0], b'b' | b'a' | b'p' | b'r' | b's')
            && rest.as_bytes()[1] == b'-'
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{scan_text_privacy, validate_workflow_uses};

    #[test]
    fn workflow_pin_validator_rejects_mutable_tags() {
        let good = "steps:\n  - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1\n";
        assert_eq!(validate_workflow_uses(good, "ci.yml"), Ok(1));
        assert!(
            validate_workflow_uses("steps:\n  - uses: actions/checkout@v4\n", "ci.yml").is_err()
        );
    }

    #[test]
    fn structural_privacy_scanner_catches_user_paths_keys_and_tokens() {
        assert!(scan_text_privacy("clean neutral text and /cargo-home/bin", "README.md").is_ok());

        let unix_path = ["", "home", "alice", "workspace"].join("/");
        assert!(scan_text_privacy(&unix_path, "doc.md").is_err());

        let mac_path = ["", "Users", "alice", "workspace"].join("/");
        assert!(scan_text_privacy(&mac_path, "doc.md").is_err());

        let pem = format!("{}EC {}", "-----BEGIN ", "PRIVATE KEY-----");
        assert!(scan_text_privacy(&pem, "key.txt").is_err());

        let ghp = format!("{}{}", "ghp_", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4");
        assert!(scan_text_privacy(&ghp, "env.txt").is_err());

        let aws = format!("{}{}", "AKIA", "IOSFODNN7EXAMPLE");
        assert!(scan_text_privacy(&aws, "env.txt").is_err());

        let slack = format!("{}{}", "xoxb-", "123456789012-abcdef");
        assert!(scan_text_privacy(&slack, "env.txt").is_err());
    }
}
