use std::sync::Arc;
use topcoat::Result;
use topcoat::router::{Response, route};
use topcoat::router::content::websocket::WebSocketUpgrade;
use topcoat::context::{Cx, app_context};

use crate::context::AppContext;
use crate::ws::handler::handle_ws_connection;

#[route(GET "/ws")]
pub async fn ws(cx: &Cx, upgrade: WebSocketUpgrade) -> Result<Response> {
    let ctx: &AppContext = app_context(cx);
    let registry = ctx.registry.clone();
    let zeroclaw = ctx.zeroclaw.clone();
    let agent = Arc::new(ctx.agent.clone());
    let whiteboard = ctx.whiteboard().cloned();

    upgrade.on_upgrade(move |socket| async move {
        handle_ws_connection(socket, registry, zeroclaw, agent, whiteboard).await;
    })
}
