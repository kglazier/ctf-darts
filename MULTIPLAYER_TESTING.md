# Multiplayer testing checklist

The four S1–S4 sessions added online play alongside the existing solo mode.
Solo is verified untouched (release build is clean, no behavior changes in
the solo code path). Online needs real-device testing because most failure
modes (NAT traversal, peer disconnect timing, snapshot drift) only show up
with live network conditions.

## Before testing

1. **Stand up the signaling server.** See `signaling/README.md`. The default
   `DEFAULT_SIGNAL_URL` in `src/net.rs` points at a public matchbox host —
   fine for first smoke test, replace before shipping.
2. **Build for Android:**
   ```bash
   cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build --release
   cd android && ./gradlew installRelease
   ```
   Don't forget the `libc++_shared.so` copy — see `project_android_build_notes`.

## Smoke tests (do these in order)

### 1. Solo still works
- Launch the app on one device.
- Tap a level + difficulty as before.
- Confirm: ship moves, bots play, captures score, game ends at target.
- **Pass = no online code path was touched.** This should look identical
  to pre-multiplayer behavior.

### 2. Lobby smoke (one device)
- Tap **PLAY ONLINE** on the menu.
- Tap **HOST GAME**. Confirm a 4-letter code appears.
- Tap **BACK**. Confirm return to menu.
- Tap **PLAY ONLINE** → **JOIN GAME**. Tap 4 letters. Tap **CONNECT**.
- Confirm "Joined lobby XXXX — waiting for host…"
- Tap **BACK**.

### 3. Two-device connect
- Device A: Host → note the code.
- Device B: Join → enter the code → Connect.
- On Device A: confirm "Connected players: 2 (you + 1)" appears within ~5
  seconds. If it doesn't, your signaling URL is wrong.
- Confirm worst-ping number appears once pings start (~2s after connect).
- Device A: tap START.
- Both devices should transition to Playing.

### 4. Gameplay sanity
- Both ships should be visible on both devices.
- Move on Device A — your ship should feel responsive locally and Device B
  should see it move smoothly (not jittery).
- Move on Device B — same in reverse.
- Boost on Device A toward Device B's ship — confirm tag works.
- Carry the flag, score — both score counters should update.

### 5. Bot fill
- In a fresh lobby, set Red bots = 1, Blue bots = 1.
- Start the game. There should be 4 ships total: 2 humans + 2 bots.
- Bots should behave the same as in solo (chase, defend, etc.).

### 6. Disconnect handling
- Close the app on Device B (the client) mid-match.
- On Device A (host): the disconnected ship should keep playing as a bot.
- Reverse: close the app on Device A (host).
- On Device B: should bounce back to the menu within a few seconds.

## Known limitations / things to validate carefully

- **Projectiles in Shooter mode are NOT replicated yet.** They'll spawn on
  the host but clients won't see them. Fix lives in S4 polish work; not on
  the critical path for Classic mode.
- **Mid-game join is forbidden.** Lobby Start locks the roster.
- **Host migration is not implemented.** If host drops, the match ends.
- **Signaling reconnection is not implemented.** If the matchbox server
  blips, peers won't auto-reconnect; they'd need to back out and re-join.

## Things to tune from observation

- `HOST_INPUT_DELAY_SECS` (`src/net.rs`, default 25ms) — bump to 35ms if
  the host wins an obvious majority of mutual tag attempts in playtests.
- `RENDER_DELAY_SECS` (`src/net.rs`, default 50ms) — increase if remote
  ships look jittery at the cost of perceived lag.
- `RECONCILE_SNAP_DIST` (`src/net.rs`, default 80) — decrease if local
  prediction visibly diverges from where the host says you should be.
- Bot AI tuning constants in `src/bot.rs` are unchanged from solo.
