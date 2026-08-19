use topcoat::Result;
use topcoat::router::page;
use topcoat::view::view;

use crate::components::chat_section::chat_section;

#[page("/")]
pub async fn home() -> Result {
    view! {
        chat_section()
    }
}
