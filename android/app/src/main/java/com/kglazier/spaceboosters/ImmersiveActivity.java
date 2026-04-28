package com.kglazier.spaceboosters;

import android.app.NativeActivity;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.NetworkRequest;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;

/**
 * Thin wrapper around NativeActivity that enables immersive sticky mode,
 * hiding the system navigation bar and status bar so the game gets
 * the full screen.
 *
 * Also binds the process to the active network so native code (libc
 * getaddrinfo from Rust / matchbox / webrtc) can actually resolve
 * hostnames. Without this, NativeActivity-based apps see DNS failures
 * (EAI_NODATA) even though Java/Chrome on the same device work fine,
 * because the C runtime resolver has no associated network.
 */
public class ImmersiveActivity extends NativeActivity {

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        hideSystemUI();
        // Stash the active network handle for native code to read via JNI
        // immediately, even before bindProcessToActiveNetwork's async path
        // resolves. If there's no active network yet it'll get refreshed
        // by tryBind() in the callbacks.
        NetworkProvider.update(this);
        bindProcessToActiveNetwork();
    }

    /**
     * Ask the OS for the active network (WiFi or cellular) and bind this
     * process to it. Required so native code can use the right DNS
     * resolver — NativeActivity does NOT do this automatically. Re-binds
     * whenever the active network changes (e.g., WiFi drops, cellular
     * takes over) so a network swap doesn't permanently break DNS.
     */
    private static final String TAG = "SpaceBoostersNet";

    private void bindProcessToActiveNetwork() {
        final ConnectivityManager cm =
            (ConnectivityManager) getSystemService(CONNECTIVITY_SERVICE);
        if (cm == null) {
            Log.w(TAG, "ConnectivityManager is null; native DNS will not work");
            return;
        }
        final Handler mainHandler = new Handler(Looper.getMainLooper());

        Network active = cm.getActiveNetwork();
        Log.i(TAG, "onCreate: active network = " + active);
        if (active != null) {
            tryBind(cm, active, "onCreate");
        }

        // registerNetworkCallback (NOT requestNetwork) — only monitor for
        // changes; requestNetwork would need CHANGE_NETWORK_STATE permission
        // which is signature-only on modern Android. The onCreate bind above
        // does the heavy lifting; this just re-binds on WiFi↔cellular hand-offs.
        NetworkRequest request = new NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .addCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
            .build();
        cm.registerNetworkCallback(
            request,
            new ConnectivityManager.NetworkCallback() {
                @Override
                public void onAvailable(final Network network) {
                    mainHandler.post(() -> tryBind(cm, network, "onAvailable"));
                }
            }
        );
    }

    private void tryBind(ConnectivityManager cm, Network network, String source) {
        boolean ok = cm.bindProcessToNetwork(network);
        Log.i(TAG, source + ": bindProcessToNetwork(" + network + ") = " + ok);
        // Always refresh NetworkProvider — at SDK 35 bindProcessToNetwork
        // may return false but per-socket binding via NDK still works,
        // and that needs the handle stashed here.
        NetworkProvider.update(ImmersiveActivity.this);
    }

    @Override
    public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) {
            hideSystemUI();
        }
    }

    private void hideSystemUI() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            // API 30+ (Android 11+): use WindowInsetsController
            getWindow().setDecorFitsSystemWindows(false);
            WindowInsetsController controller = getWindow().getInsetsController();
            if (controller != null) {
                controller.hide(WindowInsets.Type.systemBars());
                controller.setSystemBarsBehavior(
                    WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                );
            }
        } else {
            // Pre-API 30: use system UI flags
            View decorView = getWindow().getDecorView();
            decorView.setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                | View.SYSTEM_UI_FLAG_FULLSCREEN
            );
        }
    }
}
