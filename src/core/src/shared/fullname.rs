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
        let nickname = if nickname.is_empty() { None } else { Some(nickname) };
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

    pub fn display_last_name(&self) -> &str {
        self.effective_nickname().unwrap_or(&self.last_name)
    }

    pub fn display_first_name(&self) -> &str {
        if self.effective_nickname().is_some() {
            ""
        } else {
            &self.first_name
        }
    }
}

impl Display for FullName {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        if let Some(nickname) = self.effective_nickname() {
            return write!(f, "{}", nickname);
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
        let fullname =
            FullName::with_nickname("Ronaldo".to_string(), "de Lima".to_string(), "Ronaldinho".to_string());

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
}
