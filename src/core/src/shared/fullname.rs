use deunicode::deunicode;
use std::fmt::{Display, Formatter, Result};

#[derive(Debug, Clone)]
pub struct FullName {
    pub first_name: String,
    pub last_name: String,
    pub middle_name: Option<String>,
    pub nickname: Option<String>,
}

impl FullName {
    pub fn new(first_name: String, last_name: String) -> Self {
        FullName {
            first_name,
            last_name,
            middle_name: None,
            nickname: None,
        }
    }

    pub fn with_full(first_name: String, last_name: String, middle_name: String) -> Self {
        FullName {
            first_name,
            last_name,
            middle_name: Some(middle_name),
            nickname: None,
        }
    }

    pub fn with_nickname(first_name: String, last_name: String, nickname: String) -> Self {
        // An empty nickname string is the same as no nickname at all — source
        // data (odb / generator tables) sometimes carries `Some("")`, which
        // would otherwise render the player as a blank name in every view
        // that goes through `display_last_name()`.
        let nickname = if nickname.is_empty() {
            None
        } else {
            Some(nickname)
        };
        FullName {
            first_name,
            last_name,
            middle_name: None,
            nickname,
        }
    }

    fn effective_nickname(&self) -> Option<&str> {
        self.nickname.as_deref().filter(|n| !n.is_empty())
    }

    // Single-name players (Brazilian convention — "Bremer", "Kaká") arrive
    // from source data with last_name="" and the mononym in first_name. Treat
    // them like nicknamed players: the mononym is the display surname, and
    // display_first_name is empty so "{first} {last}" doesn't emit a leading
    // space.
    fn is_mononym(&self) -> bool {
        self.last_name.is_empty() && !self.first_name.is_empty()
    }

    pub fn display_last_name(&self) -> &str {
        if let Some(nick) = self.effective_nickname() {
            return nick;
        }
        if self.is_mononym() {
            return &self.first_name;
        }
        &self.last_name
    }

    pub fn display_first_name(&self) -> &str {
        if self.effective_nickname().is_some() || self.is_mononym() {
            ""
        } else {
            &self.first_name
        }
    }

    /// ASCII-folded, dash-joined slug of the display name. Nickname players
    /// (Ronaldinho) and mononyms (Bremer) produce a single-segment slug;
    /// standard "first last" names produce two segments. Returns an empty
    /// string only if every source character was unprintable after folding —
    /// callers combine this with the numeric id so the URL always resolves.
    pub fn slug(&self) -> String {
        let first = self.display_first_name();
        let last = self.display_last_name();

        let mut name = String::with_capacity(first.len() + last.len() + 1);
        name.push_str(first);
        if !first.is_empty() && !last.is_empty() {
            name.push(' ');
        }
        name.push_str(last);

        slug_from_display(&name)
    }
}

/// Produce a URL-safe slug from an arbitrary display name string.
/// Shares the exact folding rules used by `FullName::slug` so slugs
/// computed from a live `Player` always match those built from historical
/// records that only kept a pre-rendered `player_name`.
pub fn slug_from_display(display: &str) -> String {
    let folded = deunicode(display);
    let mut out = String::with_capacity(folded.len());
    let mut prev_dash = true;
    for ch in folded.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    out
}

impl Display for FullName {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        if let Some(nickname) = self.effective_nickname() {
            return write!(f, "{}", nickname);
        }
        if self.is_mononym() {
            return write!(f, "{}", self.first_name);
        }
        let mut name = format!("{} {}", self.last_name, self.first_name);
        if let Some(middle_name) = self.middle_name.as_ref() {
            name.push_str(" ");
            name.push_str(middle_name);
        }
        write!(f, "{}", name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_fullname() {
        let fullname = FullName::new("John".to_string(), "Doe".to_string());

        assert_eq!(fullname.first_name, "John");
        assert_eq!(fullname.last_name, "Doe");
        assert_eq!(fullname.middle_name, None);
    }

    #[test]
    fn test_with_full_fullname() {
        let fullname =
            FullName::with_full("John".to_string(), "Doe".to_string(), "Smith".to_string());

        assert_eq!(fullname.first_name, "John");
        assert_eq!(fullname.last_name, "Doe");
        assert_eq!(fullname.middle_name, Some("Smith".to_string()));
    }

    #[test]
    fn test_display_without_middle_name() {
        let fullname = FullName::new("John".to_string(), "Doe".to_string());

        assert_eq!(format!("{}", fullname), "Doe John");
    }

    #[test]
    fn test_display_with_middle_name() {
        let fullname =
            FullName::with_full("John".to_string(), "Doe".to_string(), "Smith".to_string());

        assert_eq!(format!("{}", fullname), "Doe John Smith");
    }

    #[test]
    fn test_with_nickname() {
        let fullname = FullName::with_nickname(
            "Ronaldo".to_string(),
            "de Lima".to_string(),
            "Ronaldinho".to_string(),
        );

        assert_eq!(format!("{}", fullname), "Ronaldinho");
        assert_eq!(fullname.display_last_name(), "Ronaldinho");
        assert_eq!(fullname.display_first_name(), "");
    }

    #[test]
    fn test_display_helpers_without_nickname() {
        let fullname = FullName::new("John".to_string(), "Doe".to_string());

        assert_eq!(fullname.display_last_name(), "Doe");
        assert_eq!(fullname.display_first_name(), "John");
    }

    #[test]
    fn test_mononym() {
        let fullname = FullName::new("Bremer".to_string(), "".to_string());

        assert_eq!(fullname.display_last_name(), "Bremer");
        assert_eq!(fullname.display_first_name(), "");
        assert_eq!(format!("{}", fullname), "Bremer");
    }

    #[test]
    fn slug_simple_latin() {
        let fn_ = FullName::new("Unai".to_string(), "Simon".to_string());
        assert_eq!(fn_.slug(), "unai-simon");
    }

    #[test]
    fn slug_folds_diacritics() {
        let fn_ = FullName::new("Iñigo".to_string(), "Martínez".to_string());
        assert_eq!(fn_.slug(), "inigo-martinez");

        let fn_ = FullName::new("Thomas".to_string(), "Müller".to_string());
        assert_eq!(fn_.slug(), "thomas-muller");
    }

    #[test]
    fn slug_handles_nickname_only() {
        let fn_ = FullName::with_nickname(
            "Ronaldo".to_string(),
            "de Lima".to_string(),
            "Ronaldinho".to_string(),
        );
        assert_eq!(fn_.slug(), "ronaldinho");
    }

    #[test]
    fn slug_handles_mononym() {
        let fn_ = FullName::new("Bremer".to_string(), "".to_string());
        assert_eq!(fn_.slug(), "bremer");

        let fn_ = FullName::new("Kaká".to_string(), "".to_string());
        assert_eq!(fn_.slug(), "kaka");
    }

    #[test]
    fn slug_collapses_punctuation() {
        let fn_ = FullName::new("Jean-Pierre".to_string(), "O'Brien".to_string());
        assert_eq!(fn_.slug(), "jean-pierre-o-brien");
    }

    #[test]
    fn slug_transliterates_cyrillic() {
        let fn_ = FullName::new("Игорь".to_string(), "Акинфеев".to_string());
        let slug = fn_.slug();
        assert!(!slug.is_empty());
        assert!(slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}
