#[derive(Debug, Clone, Copy)]
pub enum GeminiModel {
    Pro,
    Flash,
}

impl GeminiModel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pro => "gemini-3-pro-preview",
            Self::Flash => "gemini-3-flash-preview",
        }
    }

    pub fn from_str(s: &str) -> Self {
        if s.contains("flash") {
            Self::Flash
        } else {
            Self::Pro
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_as_str() {
        assert_eq!(GeminiModel::Pro.as_str(), "gemini-3-pro-preview");
        assert_eq!(GeminiModel::Flash.as_str(), "gemini-3-flash-preview");
    }

    #[test]
    fn test_model_from_str() {
        assert!(matches!(GeminiModel::from_str("pro"), GeminiModel::Pro));
        assert!(matches!(
            GeminiModel::from_str("flash"),
            GeminiModel::Flash
        ));
        assert!(matches!(
            GeminiModel::from_str("gemini-flash"),
            GeminiModel::Flash
        ));
        assert!(matches!(
            GeminiModel::from_str("anything else"),
            GeminiModel::Pro
        ));
    }
}
