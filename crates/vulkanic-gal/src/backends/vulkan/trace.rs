//! Tracy zones and messages. Without the `tracy` feature these compile to
//! nothing; with it, they record only when `VULKANIC_TRACY=1` (read once).

pub(super) use crate::diagnostics::{stderr, stdout};

#[cfg(feature = "tracy")]
use std::sync::OnceLock;

#[cfg(feature = "tracy")]
static CLIENT: OnceLock<tracy_client::Client> = OnceLock::new();

#[cfg(feature = "tracy")]
fn client() -> &'static tracy_client::Client {
    CLIENT.get_or_init(|| {
        let client = tracy_client::Client::start();
        stderr(format_args!("VulkanicGAL Tracy client start requested"));
        client.set_thread_name("VulkanicGAL");
        client.message("VulkanicGAL Tracy client started", 0);
        client
    })
}

#[cfg(feature = "tracy")]
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        crate::env::var("VULKANIC_TRACY")
            .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
    })
}

/// A Tracy zone open until dropped.
pub(super) struct Zone {
    #[cfg(feature = "tracy")]
    _span: Option<tracy_client::Span>,
}

impl Zone {
    #[cfg(not(feature = "tracy"))]
    #[inline(always)]
    pub(super) fn new(_name: &'static str) -> Self {
        Self {}
    }

    #[cfg(feature = "tracy")]
    pub(super) fn new(name: &'static str) -> Self {
        if !enabled() {
            return Self { _span: None };
        }
        let client = client();
        client.message(name, 0);
        Self {
            _span: Some(client.clone().span_alloc(
                Some(name),
                "vulkanic_gal::backends::vulkan",
                file!(),
                line!(),
                0,
            )),
        }
    }
}

/// A Tracy message; formatted only when Tracy records.
#[cfg(not(feature = "tracy"))]
#[inline(always)]
pub(super) fn message(_message: std::fmt::Arguments<'_>) {}

/// A Tracy message; formatted only when Tracy records.
#[cfg(feature = "tracy")]
pub(super) fn message(message: std::fmt::Arguments<'_>) {
    if enabled() {
        client().message(&message.to_string(), 0);
    }
}
