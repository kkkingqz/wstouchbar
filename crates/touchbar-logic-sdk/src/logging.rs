#[cfg(target_arch = "wasm32")]
struct GuestLogger;
#[cfg(target_arch = "wasm32")]
impl log::Log for GuestLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        use crate::bindings::LogLevel;
        let level = match record.level() {
            log::Level::Trace => LogLevel::Trace,
            log::Level::Debug => LogLevel::Debug,
            log::Level::Info => LogLevel::Info,
            log::Level::Warn => LogLevel::Warn,
            log::Level::Error => LogLevel::Error,
        };
        crate::bindings::log(level, &record.args().to_string());
    }
    fn flush(&self) {}
}

/// Called by the exported bridge; native author tests keep their existing logger.
#[doc(hidden)]
pub fn init_logging() {
    #[cfg(target_arch = "wasm32")]
    {
        static LOGGER: GuestLogger = GuestLogger;
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Trace);
        }
    }
}
