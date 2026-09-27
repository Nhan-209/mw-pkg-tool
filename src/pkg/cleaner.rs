use anyhow::Result;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use walkdir::WalkDir;

pub fn clean_lua_content(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    let mut byte_seq = Vec::new();
    let mut raw_escape_spans = Vec::new();

    while i < len {
        // Check for decompiler comments like "-- pseudo-goto"
        if bytes[i..].starts_with(b"-- pseudo-goto") {
            i += b"-- pseudo-goto".len();
            continue;
        }

        // Check for decimal escape sequence starting with '\' followed by digits
        if bytes[i] == b'\\' && i + 1 < len && bytes[i + 1].is_ascii_digit() {
            byte_seq.clear();
            raw_escape_spans.clear();

            let mut cur = i;
            while cur < len && bytes[cur] == b'\\' && cur + 1 < len && bytes[cur + 1].is_ascii_digit() {
                let start_esc = cur;
                cur += 1;
                let mut num: u32 = 0;
                let mut digit_count = 0;
                while cur < len && bytes[cur].is_ascii_digit() && digit_count < 3 {
                    num = num * 10 + (bytes[cur] - b'0') as u32;
                    cur += 1;
                    digit_count += 1;
                }

                if num <= 255 {
                    byte_seq.push(num as u8);
                    raw_escape_spans.push(start_esc..cur);
                } else {
                    // Invalid byte value, rewind
                    cur = start_esc;
                    break;
                }
            }

            if !byte_seq.is_empty() {
                if let Ok(decoded_str) = std::str::from_utf8(&byte_seq) {
                    for ch in decoded_str.chars() {
                        match ch {
                            '\\' => out.push_str("\\\\"),
                            '"' => out.push_str("\\\""),
                            '\n' => out.push_str("\\n"),
                            '\r' => out.push_str("\\r"),
                            '\t' => out.push_str("\\t"),
                            _ => out.push(ch),
                        }
                    }
                    i = cur;
                    continue;
                } else {
                    // Not valid UTF-8, output raw original escapes
                    let total_span_end = raw_escape_spans.last().unwrap().end;
                    let raw_str = std::str::from_utf8(&bytes[i..total_span_end]).unwrap_or_default();
                    out.push_str(raw_str);
                    i = total_span_end;
                    continue;
                }
            }
        }

        // Normalize CRLF to LF
        if bytes[i] == b'\r' && i + 1 < len && bytes[i + 1] == b'\n' {
            out.push('\n');
            i += 2;
            continue;
        }

        out.push(bytes[i] as char);
        i += 1;
    }

    out
}

pub fn clean_directory<P: AsRef<Path>, Q: AsRef<Path>>(
    input_dir: P,
    output_dir: Option<Q>,
) -> Result<(usize, std::time::Duration)> {
    let input_dir = input_dir.as_ref();
    let out_dir = match output_dir {
        Some(d) => d.as_ref().to_path_buf(),
        None => input_dir.to_path_buf(),
    };

    let timer = Instant::now();
    println!("[*] Cleaning and decoding Lua files in: {:?}", input_dir);

    let mut count = 0;
    let mut in_place = input_dir == out_dir;

    for entry in WalkDir::new(input_dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            let is_lua = path.extension().map(|ext| ext.eq_ignore_ascii_case("lua")).unwrap_or(false);
            if is_lua {
                let mut content = String::new();
                let mut f = File::open(path)?;
                f.read_to_string(&mut content)?;

                let cleaned = clean_lua_content(&content);

                let target_path = if in_place {
                    path.to_path_buf()
                } else {
                    let rel = path.strip_prefix(input_dir)?;
                    let dest = out_dir.join(rel);
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    dest
                };

                let mut out_f = File::create(&target_path)?;
                out_f.write_all(cleaned.as_bytes())?;
                count += 1;

                if count % 1000 == 0 {
                    println!("    - Cleaned {} files...", count);
                }
            } else if !in_place {
                // Copy non-lua files as-is
                let rel = path.strip_prefix(input_dir)?;
                let dest = out_dir.join(rel);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(path, dest)?;
            }
        }
    }

    let elapsed = timer.elapsed();
    println!(
        "[+] Successfully cleaned {} Lua files in {:.2?}",
        count, elapsed
    );

    Ok((count, elapsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_lua_escapes() {
        let input = r#"local name = "\230\156\170\231\159\165" .. "\229\144\137\230\158\151" -- pseudo-goto"#;
        let expected = r#"local name = "未知" .. "吉林" "#;
        let cleaned = clean_lua_content(input);
        assert_eq!(cleaned, expected);
    }
}
