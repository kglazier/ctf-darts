//! Phase 1 validation: prove that pre-binding a TCP socket via the NDK
//! `android_setsocknetwork()` lets it reach an external host even when
//! `bindProcessToNetwork()` is broken (Android 15 + targetSdk 35).
//!
//! What it does at app startup:
//!   1. Wait a moment for the Java side to populate the network handle.
//!   2. Pull the handle into our cache via `refresh_from_java()`.
//!   3. Create a `tokio::net::TcpSocket`, grab its raw fd, and bind it
//!      to the active Android Network with `bind_fd`.
//!   4. Connect to the fly signaling server's IPv6 IP on port 80.
//!   5. Log success or failure to logcat — that's the whole signal.
//!
//! On non-Android targets this module is a no-op so it doesn't drag the
//! desktop build into pulling tokio's network stack at runtime.

use bevy::prelude::*;

pub struct NetSmokeTestPlugin;
impl Plugin for NetSmokeTestPlugin {
    fn build(&self, app: &mut App) {
        // Run once a few seconds after startup so the Java side has time
        // to receive the active-network callback and stash the handle.
        app.add_systems(Startup, schedule_smoke_test);
    }
}

fn schedule_smoke_test(_commands: Commands) {
    #[cfg(target_os = "android")]
    {
        std::thread::spawn(|| {
            // Give the activity a couple of seconds to wire networks up.
            std::thread::sleep(std::time::Duration::from_secs(3));
            run_smoke_test();
        });
    }
}

#[cfg(target_os = "android")]
fn run_smoke_test() {
    use std::os::fd::AsRawFd;
    use std::time::Duration;

    let target = "[2a09:8280:1::10b:a1c3:0]:80";

    // Pull the handle from Java's NetworkProvider.
    let have_handle = android_net_bind::refresh_from_java();
    bevy::log::info!(
        "smoke-test: refresh_from_java -> {} (cached handle = {:?})",
        have_handle,
        android_net_bind::active_network_handle()
    );

    // Build a tiny tokio runtime just for this one async connect.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            bevy::log::warn!("smoke-test: failed to build tokio rt: {e}");
            return;
        }
    };

    rt.block_on(async {
        let socket = match tokio::net::TcpSocket::new_v6() {
            Ok(s) => s,
            Err(e) => {
                bevy::log::warn!("smoke-test: TcpSocket::new_v6 failed: {e}");
                return;
            }
        };
        let fd = socket.as_raw_fd();
        match android_net_bind::bind_fd(fd) {
            Ok(()) => bevy::log::info!("smoke-test: bind_fd OK"),
            Err(e) => bevy::log::warn!("smoke-test: bind_fd FAILED: {e}"),
        }

        let addr: std::net::SocketAddr = match target.parse() {
            Ok(a) => a,
            Err(e) => {
                bevy::log::warn!("smoke-test: target parse failed: {e}");
                return;
            }
        };

        match tokio::time::timeout(Duration::from_secs(5), socket.connect(addr)).await {
            Ok(Ok(_stream)) => {
                bevy::log::info!("smoke-test: CONNECTED to {target} — NDK bind works ✓");
            }
            Ok(Err(e)) => {
                bevy::log::warn!("smoke-test: connect failed: {e}");
            }
            Err(_) => {
                bevy::log::warn!("smoke-test: connect timed out after 5s");
            }
        }
    });
}
