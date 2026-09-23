//! Resource limits shared by isolated parser and archive workers.
pub(crate) fn limit_memory() {
    // Hard memory ceiling for foreign parsers. Linux uses virtual address
    // space; macOS enforces data size. No libraries or runtime tools required.
    unsafe {
        let limit = libc::rlimit {
            rlim_cur: 768 * 1024 * 1024,
            rlim_max: 768 * 1024 * 1024,
        };
        #[cfg(target_os = "linux")]
        libc::setrlimit(libc::RLIMIT_AS, &limit);
        #[cfg(target_os = "macos")]
        libc::setrlimit(libc::RLIMIT_DATA, &limit);
    }
}
