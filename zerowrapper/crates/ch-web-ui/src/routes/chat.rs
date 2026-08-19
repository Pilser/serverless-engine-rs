use topcoat::Result;
use topcoat::router::page;
use topcoat::view::view;

use crate::components::chat_section::chat_section;

#[page("/chat")]
pub async fn chat() -> Result {
    view! {
        chat_section()
    }
}
