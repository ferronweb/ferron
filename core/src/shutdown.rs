use std::sync::LazyLock;

use arc_swap::ArcSwap;
use tokio_util::sync::CancellationToken;

pub static SHUTDOWN_TOKEN: LazyLock<ArcSwap<CancellationToken>> =
    LazyLock::new(|| ArcSwap::from_pointee(CancellationToken::new()));
pub static RELOAD_TOKEN: LazyLock<ArcSwap<CancellationToken>> =
    LazyLock::new(|| ArcSwap::from_pointee(CancellationToken::new()));
