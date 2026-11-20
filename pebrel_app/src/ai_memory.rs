//! User-authored AI preferences shared across documents and built-in AI inference sessions.
//!
//! This store contains preferences only. Document bodies and transcripts are never
//! copied into it automatically.

use std::io;
use std::path::PathBuf;

const FILE_NAME: &str = "ai-habits.md";
const MAX_BYTES: usize = 16 * 1024;
const MAX_CHARS: usize = 4_000;

fn path() -> PathBuf {
    crate::platform::dirs::data_dir().join(FILE_NAME)
}

pub fn load() -> String {
    let Ok(bytes) = std::fs::read(path()) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_BYTES)]);
    normalize(&text).chars().take(MAX_CHARS).collect()
}

pub fn save(value: &str) -> io::Result<()> {
    let normalized = normalize(value);
    if normalized.chars().count() > MAX_CHARS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("AI preferences must be {MAX_CHARS} characters or fewer"),
        ));
    }
    crate::atomic_file::write_private(&path(), normalized.as_bytes())
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\r' | '\t'))
        .take(MAX_CHARS + 1)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_text_is_bounded_and_preserves_multiline_formatting() {
        assert_eq!(normalize("  concise\nChinese answers\t  "), "concise\nChinese answers");
        assert_eq!(normalize(&"x".repeat(MAX_CHARS + 10)).chars().count(), MAX_CHARS + 1);
    }
}
