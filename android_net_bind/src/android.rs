//! Android-only NDK FFI for per-socket network binding. Wraps the C
//! functions in `android/multinetwork.h`, available since API 23.
//!
//! Linker note: these symbols live in `libandroid.so`, which is part of
//! every Android NDK toolchain. We tell cargo to link it via the
//! `#[link]` attribute on the extern block — no build.rs needed.

use std::ffi::CString;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::os::raw::{c_char, c_int};
use std::ptr;

use libc::{addrinfo, freeaddrinfo, sockaddr, sockaddr_in, sockaddr_in6, AF_INET, AF_INET6};

use jni::objects::JClass;
use jni::JavaVM;

/// `net_handle_t` from `android/multinetwork.h`. Same bit pattern as
/// Java's `Network.getNetworkHandle()` returns.
type NetHandle = u64;

#[link(name = "android")]
extern "C" {
    fn android_setsocknetwork(network: NetHandle, fd: c_int) -> c_int;
    fn android_getaddrinfofornetwork(
        network: NetHandle,
        node: *const c_char,
        service: *const c_char,
        hints: *const addrinfo,
        res: *mut *mut addrinfo,
    ) -> c_int;
}

pub(crate) fn bind_fd_inner(handle: NetHandle, fd: c_int) -> io::Result<()> {
    let rc = unsafe { android_setsocknetwork(handle, fd) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn resolve_inner(handle: NetHandle, host: &str, port: u16) -> io::Result<SocketAddr> {
    // Build CStrings for getaddrinfo's node + service args.
    let host_c = CString::new(host)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let port_str = port.to_string();
    let port_c = CString::new(port_str)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

    let mut res: *mut addrinfo = ptr::null_mut();
    let rc = unsafe {
        android_getaddrinfofornetwork(
            handle,
            host_c.as_ptr(),
            port_c.as_ptr(),
            ptr::null(), // hints — let the system pick AF_UNSPEC
            &mut res,
        )
    };
    if rc != 0 {
        // Per getaddrinfo convention, rc is an EAI_* code, not errno.
        // Map both directions for a useful error message.
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("android_getaddrinfofornetwork failed: rc={rc}"),
        ));
    }
    if res.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "no addresses returned",
        ));
    }

    // Walk the linked list, return the first IPv4/IPv6 we can decode.
    let chosen: Option<SocketAddr> = unsafe {
        let mut cur = res;
        let mut found = None;
        while !cur.is_null() {
            let info = &*cur;
            let sa = info.ai_addr as *const sockaddr;
            if !sa.is_null() {
                match (*sa).sa_family as i32 {
                    AF_INET => {
                        let v4 = &*(sa as *const sockaddr_in);
                        let ip = Ipv4Addr::from(u32::from_be(v4.sin_addr.s_addr));
                        let p = u16::from_be(v4.sin_port);
                        found = Some(SocketAddr::new(IpAddr::V4(ip), p));
                        break;
                    }
                    AF_INET6 => {
                        let v6 = &*(sa as *const sockaddr_in6);
                        let ip = Ipv6Addr::from(v6.sin6_addr.s6_addr);
                        let p = u16::from_be(v6.sin6_port);
                        found = Some(SocketAddr::new(IpAddr::V6(ip), p));
                        break;
                    }
                    _ => {}
                }
            }
            cur = info.ai_next;
        }
        freeaddrinfo(res);
        found
    };

    chosen.ok_or_else(|| {
        io::Error::new(io::ErrorKind::AddrNotAvailable, "no usable address family")
    })
}

/// Pull the latest active-network handle from Java's
/// `com.kglazier.spaceboosters.NetworkProvider.getActiveHandle()`. Returns
/// `Some(handle)` when Java has stashed one; `None` if no network is
/// active yet or the JNI call fails.
///
/// This is intended to be called periodically (e.g. on each new socket
/// creation) so we always pick up the freshest handle even after a
/// WiFi↔cellular handoff. The Java side updates its cached value via
/// network-change callbacks in ImmersiveActivity.
pub fn fetch_active_handle_from_java() -> Option<u64> {
    let ctx = ndk_context::android_context();
    let vm_raw = ctx.vm();
    if vm_raw.is_null() {
        return None;
    }
    let vm = unsafe { JavaVM::from_raw(vm_raw.cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;

    // Class is the fully-qualified slash-form of the Java class name.
    let class: JClass = env
        .find_class("com/kglazier/spaceboosters/NetworkProvider")
        .ok()?;
    let handle = env
        .call_static_method(class, "getActiveHandle", "()J", &[])
        .and_then(|v| v.j())
        .ok()?;
    if handle == 0 {
        None
    } else {
        Some(handle as u64)
    }
}
