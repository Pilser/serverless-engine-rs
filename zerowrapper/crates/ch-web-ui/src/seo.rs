use serde_json::json;

use crate::config::{Business, LandingData, SiteInfo};

fn branch_jsonld(branch: &crate::config::BusinessBranch) -> serde_json::Value {
    let mut node = json!({
        "@type": "BarOrPub",
        "name": branch.name,
        "address": {
            "@type": "PostalAddress",
            "streetAddress": branch.address,
        },
    });
    if let Some(tel) = &branch.telephone {
        node["telephone"] = json!(tel);
    }
    if let Some(hours) = &branch.hours {
        node["openingHours"] = json!(hours);
    }
    if let Some(url) = &branch.url {
        node["url"] = json!(url);
    }
    if branch.latitude.is_some() && branch.longitude.is_some() {
        node["geo"] = json!({
            "@type": "GeoCoordinates",
            "latitude": branch.latitude,
            "longitude": branch.longitude,
        });
    }
    node
}

fn absolute_url(site: &SiteInfo, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    let base = site.url.trim_end_matches('/');
    let suffix = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    format!("{base}{suffix}")
}

pub fn organization_jsonld(site: &SiteInfo) -> serde_json::Value {
    json!({
        "@context": "https://schema.org",
        "@type": "Organization",
        "name": site.brand_or_title(),
        "url": site.url,
        "description": site.description,
        "logo": site.favicon.as_deref().map(|f| absolute_url(site, f)),
    })
}

pub fn business_jsonld(business: &Business, site: &SiteInfo) -> serde_json::Value {
    let biz_type = business.r#type.as_deref().unwrap_or("BarOrPub");
    let mut branches: Vec<serde_json::Value> =
        business.branches.iter().map(branch_jsonld).collect();
    if branches.is_empty() {
        branches = vec![json!({
            "@type": biz_type,
            "name": business.name.as_deref().unwrap_or_else(|| site.brand_or_title()),
            "telephone": business.telephone,
        })];
    }
    let node = json!({
        "@context": "https://schema.org",
        "@type": biz_type,
        "name": business.name.as_deref().unwrap_or_else(|| site.brand_or_title()),
        "url": site.url,
        "description": business.description.as_deref().unwrap_or(&site.description),
        "image": business.image.as_deref().map(|i| absolute_url(site, i)),
        "telephone": business.telephone,
        "branch": branches,
    });
    node
}

pub fn faq_jsonld(landing: &LandingData) -> Option<serde_json::Value> {
    if landing.faq.is_empty() {
        return None;
    }
    let items: Vec<serde_json::Value> = landing
        .faq
        .iter()
        .map(|item| {
            json!({
                "@type": "Question",
                "name": item.q,
                "acceptedAnswer": {
                    "@type": "Answer",
                    "text": item.a,
                },
            })
        })
        .collect();
    Some(json!({
        "@context": "https://schema.org",
        "@type": "FAQPage",
        "mainEntity": items,
    }))
}
