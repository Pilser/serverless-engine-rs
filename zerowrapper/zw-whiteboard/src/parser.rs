#[derive(Debug, Clone)]
pub struct ParsedMarkdown {
    pub body_md: String,
    pub pin_paths: Vec<String>,
}

pub fn parse_markdown(raw: &str) -> ParsedMarkdown {
    let mut body = Vec::new();
    let mut pin_paths = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("@pin ") {
            let path = rest.trim().to_string();
            if !path.is_empty() {
                pin_paths.push(path);
                continue;
            }
        }
        body.push(line);
    }
    ParsedMarkdown {
        body_md: body.join("\n"),
        pin_paths,
    }
}

pub fn extract_inline_images(body_md: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut rest = body_md;
    while let Some(start) = rest.find("![") {
        let after = &rest[start + 2..];
        if let Some(end_alt) = after.find(']') {
            if let Some(open_paren) = after[end_alt + 1..].find('(') {
                let paren_start = end_alt + 1 + open_paren + 1;
                if let Some(close_paren) = after[paren_start..].find(')') {
                    let path = after[paren_start..paren_start + close_paren].trim();
                    if !path.is_empty() && !path.starts_with("http") {
                        paths.push(path.to_string());
                    }
                    rest = &after[paren_start + close_paren..];
                    continue;
                }
            }
        }
        rest = &after;
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_pin_lines() {
        let parsed = parse_markdown("# Title\n\nnotes\n\n@pin diagram.png\n@pin report.pdf\n");
        assert_eq!(parsed.body_md, "# Title\n\nnotes\n");
        assert_eq!(parsed.pin_paths, vec!["diagram.png", "report.pdf"]);
    }

    #[test]
    fn pin_prefix_requires_space() {
        let parsed = parse_markdown("@pinned is not a pin\n@pin ok.png\n");
        assert_eq!(parsed.pin_paths, vec!["ok.png"]);
        assert!(parsed.body_md.contains("@pinned"));
    }

    #[test]
    fn extracts_inline_images() {
        let paths = extract_inline_images("see ![](a.png) and ![](sub/b.jpg) and ![](https://x.com/c.png)");
        assert_eq!(paths, vec!["a.png", "sub/b.jpg"]);
    }
}
