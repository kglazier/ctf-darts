package com.kglazier.spaceboosters;

import android.content.Context;
import android.net.ConnectivityManager;
import android.net.Network;
import android.util.Log;

/**
 * Bridge for native (Rust) code to fetch the active Android Network's
 * `getNetworkHandle()` value. The handle is the same uint64 the NDK's
 * android_setsocknetwork() and android_getaddrinfofornetwork() expect.
 *
 * Why we need this: starting around Android 15 / targetSdk 35,
 * ConnectivityManager.bindProcessToNetwork() returns false in some
 * configurations and native sockets stop being routable. The supported
 * fix is to bind every individual socket via the NDK functions, which
 * need this handle.
 *
 * Usage: ImmersiveActivity calls update(this) on create and on network
 * change. Native code calls getActiveHandle() (via JNI) at any time;
 * returns 0 if no network is active yet.
 */
public class NetworkProvider {
    private static final String TAG = "SpaceBoostersNet";

    /** 0 means "no active network yet" — handle 0 is reserved by NDK. */
    private static volatile long activeHandle = 0L;

    public static void update(Context ctx) {
        ConnectivityManager cm =
            (ConnectivityManager) ctx.getSystemService(Context.CONNECTIVITY_SERVICE);
        if (cm == null) {
            return;
        }
        Network net = cm.getActiveNetwork();
        if (net == null) {
            activeHandle = 0L;
            return;
        }
        long h = net.getNetworkHandle();
        activeHandle = h;
        Log.i(TAG, "NetworkProvider.update(): handle=" + h);
    }

    /** Called from Rust via JNI. See android_net_bind crate. */
    public static long getActiveHandle() {
        return activeHandle;
    }
}
