pub fn private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unicast_link_local()
                || v6.is_unspecified()
                || v6.is_multicast()
        }
    }
}

pub fn valid_url(url: &str) -> bool {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return false;
    }
    let Some(rest) = url.split_once("://").map(|(_, r)| r) else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or("").split('@').last().unwrap_or("");
    let host = host.trim_end_matches(']');
    let host = host.split(':').next().unwrap_or("").trim_matches(|c| c == '[' || c == ']');
    if host.is_empty() {
        return false;
    }
    if host.to_lowercase() == "localhost" {
        return false;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if private_ip(ip) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::valid_url;

    #[test]
    fn valid_url_blocks_private_targets() {
        assert!(valid_url("https://example.com/hook"));
        assert!(valid_url("http://example.com:8080/x"));
        assert!(!valid_url("http://localhost/hook"));
        assert!(!valid_url("http://127.0.0.1/hook"));
        assert!(!valid_url("http://10.0.0.5:80/x"));
        assert!(!valid_url("http://192.168.1.10/x"));
        assert!(!valid_url("http://172.16.0.1/x"));
        assert!(!valid_url("http://169.254.169.254/latest/meta-data/"));
        assert!(!valid_url("http://0.0.0.0/x"));
        assert!(!valid_url("https://[::1]/x"));
        assert!(!valid_url("ftp://example.com/x"));
        assert!(!valid_url("example.com/x"));
    }

    #[test]
    fn valid_url_allows_public_targets() {
        assert!(valid_url("http://8.8.8.8/x"));
        assert!(valid_url("https://[2001:db8::1]/x"));
        assert!(valid_url("https://user:pass@example.com/x"));
        assert!(valid_url("http://example.com:8443/x"));
    }

    #[test]
    fn valid_url_rejects_malformed() {
        assert!(!valid_url(""));
        assert!(!valid_url("https://"));
        assert!(!valid_url("https:///x"));
        assert!(!valid_url("http://:8080/x"), "empty host rejected");
    }
}
