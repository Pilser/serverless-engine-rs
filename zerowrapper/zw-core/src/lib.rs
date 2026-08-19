pub mod config;
pub mod error;
pub mod types;
pub mod traits;

pub use config::WrapperConfig;
pub use error::WrapperError;
pub use types::{ChannelMessage, FileHash, Attachment, StreamEvent, HistoryEntry, SessionMessagesResult};
pub use traits::{McpToolProvider, ToolOutput, Channel, ChannelContext, ChannelRegistryHandle, ZeroClawHandle, ZeroClawCommand};
