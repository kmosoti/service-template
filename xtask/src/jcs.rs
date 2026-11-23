//! RFC 8785 JSON Canonicalization Scheme (JCS) serializer for cryptographic digests.

use serde_json::Value;

const MAX_SAFE_INTEGER: u64 = (1u64 << 53) - 1;

pub fn to_jcs_bytes(value: &Value) -> Result<Vec<u8>, String> {
    let mut out = String::new();
    write_jcs_value(value, &mut out)?;
    Ok(out.into_bytes())
}

fn write_jcs_value(value: &Value, out: &mut String) -> Result<(), String> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(num) => {
            if let Some(u) = num.as_u64() {
                if u > MAX_SAFE_INTEGER {
                    return Err(format!(
                        "integer {u} exceeds IEEE-754 JCS safe integer bound"
                    ));
                }
                out.push_str(&u.to_string());
            } else if let Some(i) = num.as_i64() {
                let min_safe = -(MAX_SAFE_INTEGER as i64);
                if i < min_safe {
                    return Err(format!("integer {i} below IEEE-754 JCS safe integer bound"));
                }
                if i == 0 {
                    out.push('0');
                } else {
                    out.push_str(&i.to_string());
                }
            } else {
                return Err(format!(
                    "non-integer JSON number `{num}` unsupported in contract JCS canonicalizer"
                ));
            }
        }
        Value::String(s) => write_jcs_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                write_jcs_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&str, &Value)> =
                map.iter().map(|(k, v)| (k.as_str(), v)).collect();
            // RFC 8785 Section 3.2.3: sort object properties by UTF-16 code unit order.
            entries.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (idx, (k, v)) in entries.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                write_jcs_string(k, out);
                out.push(':');
                write_jcs_value(v, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_jcs_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\u{0008}' => out.push_str("\\b"),
            '\u{0009}' => out.push_str("\\t"),
            '\u{000A}' => out.push_str("\\n"),
            '\u{000C}' => out.push_str("\\f"),
            '\u{000D}' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::to_jcs_bytes;

    #[test]
    fn sorts_keys_by_utf16_code_units_rather_than_utf8_bytes() {
        // U+E000 -> UTF-8 [EE 80 80], UTF-16 [E000]
        // U+10000 -> UTF-8 [F0 90 80 80], UTF-16 [D800 DC00]
        // UTF-16 order requires U+10000 (`0xD800`) BEFORE U+E000 (`0xE000`)!
        let val = json!({
            "\u{E000}": 2,
            "\u{10000}": 1,
            "a/b": "\n\t\u{0001}"
        });
        let jcs = String::from_utf8(to_jcs_bytes(&val).expect("jcs")).unwrap();
        assert_eq!(
            jcs,
            "{\"a/b\":\"\\n\\t\\u0001\",\"\u{10000}\":1,\"\u{E000}\":2}"
        );

        // Reject integers above 2^53 - 1.
        let unsafe_int = json!({ "n": 1u64 << 54 });
        assert!(to_jcs_bytes(&unsafe_int).is_err());
    }
}
