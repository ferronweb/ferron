fn panic_hook(panic_info: &std::panic::PanicHookInfo) {
    let payload_any = panic_info.payload();
    let payload: Option<&str> = if let Some(s) = payload_any.downcast_ref::<&str>() {
        Some(s)
    } else if let Some(s) = payload_any.downcast_ref::<String>() {
        Some(s)
    } else {
        None
    };

    if !ferron_core::logging::is_init()
        && ferron_core::logging::init_stdio_logger(ferron_core::logging::LogLevel::Error).is_err()
    {
        eprintln!(
            "Ferron web server just crashed (failed to init the logger): {} (at {})",
            payload.unwrap_or("<unknown crash>"),
            panic_info
                .location()
                .unwrap_or(std::panic::Location::caller())
        );
        return;
    }

    ferron_core::log_error!(
        "Ferron web server just crashed (!): {} (at {})",
        payload.unwrap_or("<unknown crash>"),
        panic_info
            .location()
            .unwrap_or(std::panic::Location::caller())
    );

    ferron_core::log_error!("Ferron version: {}", crate::build::PKG_VERSION);
    ferron_core::log_error!("Build target: {}", crate::build::BUILD_TARGET);

    ferron_core::log_error!(
        "If you believe it's a bug, please report it at \
        https://github.com/ferronweb/ferron/issues/new. Consider sharing the build \
        information (as shown above)."
    );
}

/// Installs a panic hook
pub fn install_panic_hook() {
    if !shadow_rs::is_debug() {
        std::panic::set_hook(Box::new(panic_hook));
    }
}
