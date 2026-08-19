use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct MultipartField {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

/// Minimal RFC-7578 `multipart/form-data` parser. Expects boundaries as
/// emitted by browsers/`fetch` FormData and returns all concrete fields in order.
pub fn parse(body: &[u8], boundary: &str) -> Result<Vec<MultipartField>, String> {
    let bound = format!("--{boundary}");
    let b = bound.as_bytes();
    let end_marker = format!("\r\n--{boundary}").into_bytes();
    let mut fields = Vec::new();
    let mut pos = 0usize;
    loop {
        let Some(start) = find_from(body, b, pos) else {
            break;
        };
        let after = start + b.len();
        let tail = body.get(after..).and_then(|r| r.get(..2));
        if tail == Some(b"--") {
            break;
        }
        if tail != Some(b"\r\n") {
            pos = after;
            continue;
        }
        let hdr_start = after + 2;
        let Some(sep) = find_from(body, b"\r\n\r\n", hdr_start) else {
            break;
        };
        let headers = parse_headers(&body[hdr_start..sep]);
        let content_start = sep + 4;
        let Some(content_end) = find_from(body, &end_marker, content_start) else {
            break;
        };
        let header_map = headers_map(&headers);
        let mut name = String::new();
        let mut filename = None;
        if let Some(cd) = header_map.get("content-disposition") {
            let (n, f) = parse_content_disposition(cd);
            name = n.unwrap_or_default();
            filename = f;
        }
        fields.push(MultipartField {
            name,
            filename,
            content_type: header_map.get("content-type").cloned(),
            data: body[content_start..content_end].to_vec(),
        });
        pos = content_end;
    }
    Ok(fields)
}

pub fn boundary_from_content_type(content_type: &str) -> Option<String> {
    for seg in content_type.split(';').skip(1) {
        let seg = seg.trim();
        if let Some(rest) = seg.strip_prefix("boundary=") {
            return Some(trim_quotes(rest).to_string());
        }
    }
    None
}

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn parse_headers(bytes: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in String::from_utf8_lossy(bytes).split("\r\n") {
        if let Some(colon) = line.find(':') {
            let k = line[..colon].trim().to_ascii_lowercase();
            let v = line[colon + 1..].trim().to_string();
            out.push((k, v));
        }
    }
    out
}

fn headers_map(headers: &[(String, String)]) -> HashMap<String, String> {
    headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn parse_content_disposition(value: &str) -> (Option<String>, Option<String>) {
    let mut name = None;
    let mut filename = None;
    for seg in value.split(';').skip(1) {
        let seg = seg.trim();
        if let Some(rest) = seg.strip_prefix("name=") {
            name = Some(trim_quotes(rest).to_string());
        } else if let Some(rest) = seg.strip_prefix("filename=") {
            filename = Some(trim_quotes(rest).to_string());
        }
    }
    (name, filename)
}

fn trim_quotes(s: &str) -> &str {
    s.trim().trim_matches('"')
}