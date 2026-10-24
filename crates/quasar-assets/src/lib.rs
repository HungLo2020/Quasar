//! Validated virtual asset paths, mounts, typed loading, dependency metadata and
//! a bounded cache.
//!
//! `quasar-assets` is synchronous and headless. It reads through an
//! [`AssetSource`], so games can mount a Cargo checkout, a Basalt `app`
//! directory, a memory fixture, or an embedded archive without an editor,
//! generated build step, or global registry. Assets do not need to live in Git.
//!
//! ```
//! use quasar_assets::{Asset, AssetLoader, AssetServer, LoadContext, MemorySource};
//! use std::sync::Arc;
//!
//! #[derive(Debug)]
//! struct Text(String);
//! impl Text {
//!     fn as_str(&self) -> &str { &self.0 }
//! }
//! impl Asset for Text {}
//!
//! struct TextLoader;
//! impl AssetLoader for TextLoader {
//!     type Asset = Text;
//!     fn load(&self, context: &LoadContext<'_>) -> Result<Text, quasar_assets::AssetError> {
//!         Ok(Text(String::from_utf8_lossy(context.bytes()).into_owned()))
//!     }
//! }
//!
//! let mut source = MemorySource::new("fixture");
//! source.insert("greeting.txt", b"hello".to_vec()).unwrap();
//! let mut server = AssetServer::new();
//! server.register(TextLoader);
//! server.mount("", Arc::new(source)).unwrap();
//! let text = server.load::<TextLoader>("greeting.txt").unwrap();
//! assert_eq!(text.as_str(), "hello");
//! ```
#[cfg(feature = "async")]
mod async_runtime;
mod error;
#[cfg(feature = "hot-reload")]
mod hot_reload;
mod loader;
mod path;
mod server;
mod source;

#[cfg(feature = "async")]
pub use async_runtime::{
    AsyncAssetServer, AsyncConfig, AsyncInvalidation, AsyncLoaded, AsyncRequest,
};
pub use error::{AssetError, AssetErrorKind};
#[cfg(feature = "hot-reload")]
pub use hot_reload::{AssetWatcher, WatchBatch, source_root, watchers_for_server};
pub use loader::{Asset, AssetLoader, Bytes, BytesLoader, LoadContext};
pub use path::AssetPath;
pub use server::{AssetId, AssetServer, CacheBudget, CacheStats, Handle, InvalidationReport};
pub use source::{AssetSource, FileSource, MemorySource};

pub(crate) use loader::ErasedLoader;
pub(crate) use source::SharedSource;
