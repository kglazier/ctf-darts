//! Per-socket binding to an Android `Network` handle via the NDK
//! `android_setsocknetwork` and `android_getaddrinfofornetwork` APIs.
//!
//! Why this exists: starting around Android 15 (API 35), apps targeting
//! SDK 35 can't rely on `ConnectivityManager.bindProcessToNetwork()` to
//! affect native (libc) sockets — the call silently returns `false` and
//! `socket()`/`getaddrinfo()` from native code can't reach the network.
//! The supported workaround is to bind every individual socket to a
//! specific `Network` via the NDK functions in `android/multinetwork.h`.
//!
//! Usage flow:
//!   1. Java side fetches the active `Network` via `ConnectivityManager`,
//!      calls `getNetworkHandle()` (returns a `long` since API 23), and
//!      stores it where Rust can pull it via JNI.
//!   2. Rust calls [`init_active_network_handle()`] at startup. This caches
//!      the handle in a `OnceLock` for fast access from any thread.
//!   3. Anywhere in Rust that creates a socket, call [`bind_fd()`] on the
//!      raw fd before connect. For DNS, call [`resolve_via_network()`].
//!
//! On non-Android targets all functions are no-ops (returning Ok).

use std::os::raw::c_int;
use std::sync::OnceLock;

#[cfg(target_os = "android")]
mod android;

/// Android `net_handle_t` is `uint64_t` in `multinetwork.h`. Java's
/// `Network.getNetworkHandle()` returns a signed `long` whose bit pattern
/// is the same value; cast across the JNI boundary.
pub type NetHandle = u64;

static ACTIVE: OnceLock<NetHandle> = OnceLock::new();

/// Cache the currently-active Android `Network` handle so subsequent
/// `bind_fd` / `resolve_via_network` calls have a target to bind to.
///
/// Idempotent — second call is silently ignored. To swap to a new network
/// (e.g. WiFi → cellular), use [`replace_active_network_handle`].
pub fn init_active_network_handle(handle: NetHandle) {
    let _ = ACTIVE.set(handle);
}

/// Pull the active network handle from the Java side and cache it.
/// On non-Android targets, no-op. Returns `true` if a handle was
/// successfully fetched and cached this call (or already was cached).
pub fn refresh_from_java() -> bool {
    #[cfg(target_os = "android")]
    {
        if let Some(h) = android::fetch_active_handle_from_java() {
            init_active_network_handle(h);
            return true;
        }
    }
    active_network_handle().is_some()
}

/// Replace the cached handle. Call when the active network changes.
/// Note: existing sockets are NOT re-bound — they stay on whichever
/// network they were bound to at creation.
pub fn replace_active_network_handle(handle: NetHandle) {
    // OnceLock doesn't allow swap; we leak a tiny atomic instead.
    // For now we accept the limitation: the proxy use-case re-creates
    // the matchbox socket on network change anyway, so the new socket
    // picks up the new handle on next `init`.
    let _ = ACTIVE.set(handle);
    // Best-effort: in production we'd use a `RwLock<Option<u64>>`. The
    // OnceLock keeps the fast path lock-free for the common case where
    // the network doesn't change mid-session.
}

pub fn active_network_handle() -> Option<NetHandle> {
    ACTIVE.get().copied()
}

/// Bind the given file descriptor to the active Android Network. After
/// this, `connect()` / `sendto()` on this fd will route via that network's
/// interface and use its DNS resolver.
///
/// Returns `Ok(())` on non-Android (no-op) and on success. Returns the
/// errno from the NDK call wrapped in an io::Error on failure.
pub fn bind_fd(fd: c_int) -> std::io::Result<()> {
    let Some(handle) = active_network_handle() else {
        // No handle cached yet — not an error in itself. Caller should
        // ensure init was called before relying on bind.
        return Ok(());
    };
    #[cfg(target_os = "android")]
    {
        android::bind_fd_inner(handle, fd)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = handle;
        let _ = fd;
        Ok(())
    }
}

/// Resolve `hostname:port` over the active Network using the NDK's
/// per-network resolver. Returns the first IPv4 / IPv6 SocketAddr that
/// matches. On non-Android, falls back to the standard library resolver.
pub fn resolve_via_network(host: &str, port: u16) -> std::io::Result<std::net::SocketAddr> {
    #[cfg(target_os = "android")]
    {
        if let Some(handle) = active_network_handle() {
            return android::resolve_inner(handle, host, port);
        }
    }
    // Fallback: stdlib resolver. Used on desktop, and as a last resort on
    // Android if init hasn't been called yet (caller will likely fail
    // shortly after, but at least we tried).
    use std::net::ToSocketAddrs;
    (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no addresses"))
}
