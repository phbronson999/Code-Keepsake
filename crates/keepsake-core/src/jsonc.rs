//! VS Code writes `.code-workspace` files as JSONC: JSON plus `//` and `/* */`
//! comments and trailing commas. This turns that into strict JSON for serde_json.
//! std-only on purpose, so it is easy to test.

/// Converts JSONC text into strict JSON. Strings (including escaped quotes and
/// `//` inside URLs) are left untouched.
pub fn to_strict_json(input: &str) -> String {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    remove_trailing_commas(&strip_comments(input))
}

fn strip_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        out.push(escaped);
                    }
                }
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                // Line comment: drop everything up to (not including) the newline.
                while let Some(&next) = chars.peek() {
                    if next == '\n' {
                        break;
                    }
                    chars.next();
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for next in chars.by_ref() {
                    if prev == '*' && next == '/' {
                        break;
                    }
                    prev = next;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

fn remove_trailing_commas(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if c == '"' {
            in_string = true;
        } else if c == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::to_strict_json;

    #[test]
    fn strips_comments_and_trailing_commas() {
        let input = r#"{
            // the main app
            "folders": [
                { "path": "../app", /* inline */ },
                { "path": "C:\\src\\tool", "name": "Tool" },
            ],
            "settings": { "url": "https://example.com/a//b" },
        }"#;
        let out = to_strict_json(input);
        assert!(!out.contains("main app"));
        assert!(!out.contains("inline"));
        assert!(out.contains(r#""https://example.com/a//b""#));
        assert!(!out.contains(",\n            ]"));
        assert!(!out.replace(char::is_whitespace, "").contains(",}"));
        assert!(!out.replace(char::is_whitespace, "").contains(",]"));
    }

    #[test]
    fn keeps_escaped_quotes_and_bom() {
        let out = to_strict_json("\u{feff}{\"a\": \"say \\\"hi\\\" // not a comment\",}");
        assert_eq!(out, "{\"a\": \"say \\\"hi\\\" // not a comment\"}");
    }
}
