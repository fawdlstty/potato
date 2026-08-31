pub(crate) fn strip_path_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() || prefix == "/" {
        return Some(path);
    }

    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return Some(path);
    }

    match path.strip_prefix(prefix) {
        Some("") => Some("/"),
        Some(suffix) if suffix.starts_with('/') => Some(suffix),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::strip_path_prefix;

    #[test]
    fn strips_only_complete_path_segments() {
        assert_eq!(strip_path_prefix("/api/user", "/api"), Some("/user"));
        assert_eq!(strip_path_prefix("/api", "/api"), Some("/"));
        assert_eq!(strip_path_prefix("/api/", "/api"), Some("/"));
        assert_eq!(strip_path_prefix("/api-v1/user", "/api"), None);
        assert_eq!(strip_path_prefix("/apifoo", "/api"), None);
    }

    #[test]
    fn handles_trailing_slash_and_root_prefixes() {
        assert_eq!(strip_path_prefix("/api/user", "/api/"), Some("/user"));
        assert_eq!(strip_path_prefix("/api", "/api/"), Some("/"));
        assert_eq!(strip_path_prefix("/api-v1", "/api/"), None);
        assert_eq!(strip_path_prefix("/api/user", "/"), Some("/api/user"));
        assert_eq!(strip_path_prefix("/", "/"), Some("/"));
        assert_eq!(strip_path_prefix("/api/user", ""), Some("/api/user"));
    }
}
