use crate::transport::rest::{board_of, err_json, ok_json, require_admin, BoxBodyResp};
use crate::Server;
use engine::policy::RateLimits;
use hyper::{Response, StatusCode};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn resource_report(server: &Server, board: &str) -> Value {
    let (records, storage) = {
        let engine = server.engine.lock().unwrap();
        let records = engine.count_records(board).unwrap_or(0);
        let mut storage: u64 = 0;
        if let Ok(keys) = engine.object_store().list(&format!("{board}/files/")) {
            storage += keys.iter().map(|k| k.size).sum::<u64>();
        }
        if let Ok(keys) = engine.object_store().list(&format!("{board}/assets/")) {
            storage += keys.iter().map(|k| k.size).sum::<u64>();
        }
        (records, storage)
    };

    let rate: RateLimits = server.rate_limits_for(&board_of(server, board));
    let usage = server.limiter.lock().unwrap().snapshot(board);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    json!({
        "ok": true,
        "timestamp": now,
        "board": board,
        "records": {
            "count": records,
        },
        "storage": {
            "bytes": storage,
            "kib": (storage / 1024),
        },
        "rate": {
            "limits": {
                "submit": rate.submit,
                "upload": rate.upload,
                "search": rate.search,
                "read": rate.read,
                "per_day": rate.per_day,
            },
            "usage": usage,
        },
        "system": {
            "in_flight": server.obs.in_flight(),
            "requests_total": server.obs.total(),
            "cpu_percent": (server.obs.cpu_percent() * 100.0).round() / 100.0,
            "mem_kb": server.obs.mem_kb(),
        },
    })
}

pub fn resources(server: &Server, board: &str, principal: &engine::Principal) -> Response<BoxBodyResp> {
    if !require_admin(principal) {
        return err_json(StatusCode::FORBIDDEN, "admin authorization required");
    }
    ok_json(resource_report(server, board))
}
