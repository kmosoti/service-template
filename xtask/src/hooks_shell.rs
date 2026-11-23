//! Quote-, subshell-, and shell-wrapper-aware compound command tokenizer for `PreToolUse` hooks.

const MAX_WRAPPER_DEPTH: usize = 4;

pub fn extract_shell_commands(command: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    collect_commands(command, 0, &mut out);
    out
}

fn collect_commands(script: &str, depth: usize, out: &mut Vec<Vec<String>>) {
    if depth > MAX_WRAPPER_DEPTH {
        return;
    }

    let (segments, nested_scripts) = lex_segments(script);
    for nested in nested_scripts {
        collect_commands(&nested, depth + 1, out);
    }

    for seg in segments {
        let normalized = normalize_invocation(seg);
        if normalized.is_empty() {
            continue;
        }
        if let Some(inner) = extract_shell_wrapper_script(&normalized) {
            collect_commands(&inner, depth + 1, out);
        }
        out.push(normalized);
    }
}

fn lex_segments(script: &str) -> (Vec<Vec<String>>, Vec<String>) {
    let mut segments: Vec<Vec<String>> = Vec::new();
    let mut nested: Vec<String> = Vec::new();
    let mut cur_seg: Vec<String> = Vec::new();
    let mut cur_tok = String::new();
    let mut token_open = false;
    let mut in_single = false;
    let mut in_double = false;

    let chars: Vec<char> = script.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];
        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                cur_tok.push(c);
            }
            i += 1;
            continue;
        }

        if c == '\\' && i + 1 < chars.len() {
            cur_tok.push(chars[i + 1]);
            token_open = true;
            i += 2;
            continue;
        }

        if !in_single && c == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
            let (sub, next_idx) = take_balanced_parens(&chars, i + 2);
            nested.push(sub);
            i = next_idx;
            continue;
        }

        if !in_single && c == '`' {
            let (sub, next_idx) = take_backticks(&chars, i + 1);
            nested.push(sub);
            i = next_idx;
            continue;
        }

        if in_double {
            if c == '"' {
                in_double = false;
            } else {
                cur_tok.push(c);
            }
            i += 1;
            continue;
        }

        match c {
            '\'' => {
                in_single = true;
                token_open = true;
            }
            '"' => {
                in_double = true;
                token_open = true;
            }
            ';' | '\n' | '&' | '|' | '(' | ')' | '{' | '}' => {
                flush_tok(&mut cur_tok, &mut token_open, &mut cur_seg);
                if !cur_seg.is_empty() {
                    segments.push(std::mem::take(&mut cur_seg));
                }
            }
            ws if ws.is_whitespace() => {
                flush_tok(&mut cur_tok, &mut token_open, &mut cur_seg);
            }
            other => {
                cur_tok.push(other);
                token_open = true;
            }
        }
        i += 1;
    }

    flush_tok(&mut cur_tok, &mut token_open, &mut cur_seg);
    if !cur_seg.is_empty() {
        segments.push(cur_seg);
    }
    (segments, nested)
}

fn flush_tok(cur_tok: &mut String, token_open: &mut bool, cur_seg: &mut Vec<String>) {
    if *token_open {
        cur_seg.push(std::mem::take(cur_tok));
        *token_open = false;
    }
}

fn take_balanced_parens(chars: &[char], start: usize) -> (String, usize) {
    let mut depth = 1usize;
    let mut buf = String::new();
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '(' => {
                depth += 1;
                buf.push('(');
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (buf, i + 1);
                }
                buf.push(')');
            }
            c => buf.push(c),
        }
        i += 1;
    }
    (buf, i)
}

fn take_backticks(chars: &[char], start: usize) -> (String, usize) {
    let mut buf = String::new();
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '`' {
            return (buf, i + 1);
        }
        buf.push(chars[i]);
        i += 1;
    }
    (buf, i)
}

fn normalize_invocation(tokens: Vec<String>) -> Vec<String> {
    let mut idx = 0usize;
    while idx < tokens.len() {
        let base = binary_basename(&tokens[idx]);
        if matches!(
            base,
            "env" | "command" | "builtin" | "exec" | "nohup" | "sudo"
        ) {
            idx += 1;
            while idx < tokens.len() && tokens[idx].starts_with('-') {
                idx += 1;
            }
            continue;
        }
        if tokens[idx].contains('=') && !tokens[idx].starts_with('-') {
            idx += 1;
            continue;
        }
        break;
    }
    let mut rest = tokens[idx..].to_vec();
    if let Some(first) = rest.first_mut() {
        *first = binary_basename(first).to_owned();
    }
    rest
}

fn extract_shell_wrapper_script(tokens: &[String]) -> Option<String> {
    let first = tokens.first()?.as_str();
    if !matches!(first, "sh" | "bash" | "zsh" | "dash" | "ksh") {
        return None;
    }
    let mut i = 1usize;
    while i < tokens.len() {
        let arg = &tokens[i];
        if arg == "-c" || (arg.starts_with('-') && !arg.starts_with("--") && arg.ends_with('c')) {
            return tokens.get(i + 1).cloned();
        }
        i += 1;
    }
    None
}

fn binary_basename(tok: &str) -> &str {
    tok.rsplit(['/', '\\']).next().unwrap_or(tok)
}
