use std::fmt;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct TopicMatcher {
    pattern: String,
    segments: Vec<String>,
}

impl TopicMatcher {
    pub fn new(pattern: &str) -> Self {
        let segments: Vec<String> = pattern
            .split('/')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        Self {
            pattern: pattern.to_string(),
            segments,
        }
    }

    pub fn matches(&self, topic: &str) -> bool {
        let topic_segments: Vec<&str> = topic.split('/').filter(|s| !s.is_empty()).collect();
        
        if self.segments.len() != topic_segments.len() && !self.segments.contains(&"#".to_string()) {
            return false;
        }

        let mut topic_iter = topic_segments.iter();
        let mut pattern_iter = self.segments.iter();

        while let Some(pattern_segment) = pattern_iter.next() {
            match pattern_segment.as_str() {
                "#" => return true, // Multi-level wildcard
                "*" => {
                    if topic_iter.next().is_none() {
                        return false;
                    }
                }
                segment => {
                    if let Some(topic_segment) = topic_iter.next() {
                        if segment != *topic_segment {
                            return false;
                        }
                    } else {
                        return false;
                    }
                }
            }
        }

        topic_iter.next().is_none()
    }
}

impl fmt::Display for TopicMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        let matcher = TopicMatcher::new("/images/png");
        assert!(matcher.matches("/images/png"));
        assert!(!matcher.matches("/images/jpg"));
    }

    #[test]
    fn test_single_level_wildcard() {
        let matcher = TopicMatcher::new("/images/*");
        assert!(matcher.matches("/images/png"));
        assert!(matcher.matches("/images/jpg"));
        assert!(!matcher.matches("/images/png/large"));
    }

    #[test]
    fn test_multi_level_wildcard() {
        let matcher = TopicMatcher::new("/images/#");
        assert!(matcher.matches("/images/png"));
        assert!(matcher.matches("/images/jpg"));
        assert!(matcher.matches("/images/png/large"));
        assert!(matcher.matches("/images/png/large/thumbnail"));
    }

    #[test]
    fn test_complex_pattern() {
        let matcher = TopicMatcher::new("/images/*/size/#");
        assert!(matcher.matches("/images/png/size/large"));
        assert!(matcher.matches("/images/jpg/size/small/thumbnail"));
        assert!(!matcher.matches("/images/png"));
        assert!(!matcher.matches("/images/png/width"));
    }
} 