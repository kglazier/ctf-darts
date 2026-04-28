# Netcode notes — known improvement paths

The current netcode (host-authoritative + client-side prediction +
extrapolated smooth reconciliation) works well for casual P2P. These
are improvements to consider if specific issues come back, ranked by
return-on-investment.

## 1. Convert per-snapshot smoothing to time-constant form (~10 min)

Right now `RECONCILE_SMOOTH_FACTOR = 0.15` is applied per snapshot. If
the snapshot rate ever varies (network hiccup, host frame drop), the
effective convergence time silently changes. Industry standard
(Source's `cl_smoothtime`, default 0.1s) uses a wall-clock time
constant instead.

Change in `client_apply_snapshot`:
```rust
const RECONCILE_TAU_SECS: f32 = 0.08;
let lerp_factor = 1.0 - (-time.delta_seconds() / RECONCILE_TAU_SECS).exp();
local_pos.lerp(extrapolated, lerp_factor)
```

Why we haven't done it: current behavior at 60Hz snapshots feels good.
Move to time-constant if/when snapshot rate ever varies in practice.

## 2. Rollback prediction (1-2 days)

**The "right" fix for any remaining sliding/turning issues.** When a
snapshot arrives:

1. Hard-snap simulation to server state (no smoothing).
2. Replay every unacknowledged user input from snapshot.host_time
   forward through `client_predict_local`'s logic.
3. Result: simulation is exactly correct, no backward yank on turns.

What this requires:
- Per-frame input history (last ~300ms of `(timestamp, PlayerInput)` tuples)
- Per-frame state snapshot (pos, vel, stamina) for the local ship
- On snapshot apply: rewind state to snapshot.host_time, set state to
  server's, replay all later inputs, leave state at "now"
- Visual smoothing is ONLY on the camera offset (Source's
  `cl_smoothtime` model), not the simulation

This is what Source / Quake / Valorant / Counter-Strike all do. The
research agents flagged it as the proper fix when "smoothing pulls
backward on turns" is felt. Casual games (Lance.gg, .io games, our
current model) usually don't bother — sliding is the accepted cost.

## 3. Decouple visual heading from authoritative position (~1 hr)

Slither.io trick: snake eyes turn instantly with input even though the
body lags. For us: rotate the ship sprite immediately on input
direction change, even if position is lagging. Cheap perceived-latency
win without changing simulation. Could pair well with #1 to mask any
remaining feel issues.

## 4. Snapshot bandwidth optimization (low priority)

Current snapshots are bincode-serialized full state every tick. Could:
- Delta-encode against last-acked snapshot per client (smaller wire format)
- Quantize positions/velocities to fixed-point (smaller wire format)
- Drop fields that haven't changed

Only worth doing if we ever target bad networks or scale players past 8.

## 5. Server-side lag compensation (~1 day)

Currently the host applies remote inputs using current host time. With
RTT, the client's input represents an action taken ~50ms ago. Source
does "lag compensation": rewind world state to where the shooter saw
it before validating the hit. For us, this would mean the host
rewinding flag/ship positions to the client's reported timestamp
before checking pickup/tag eligibility. Eliminates the
"I touched the flag but the host disagreed" class of bug.

Adds complexity (need to keep ~200ms history of every entity on host).
Worth doing if pickup/tag mispredictions become common.

## Reference reading

- Glenn Fiedler — gafferongames.com (especially "State Synchronization")
- Gabriel Gambetta — gabrielgambetta.com "Fast-Paced Multiplayer" series
- Yahn Bernier 2001 — "Latency Compensating Methods" (Valve)
- Lance.gg docs — closest analog to our architecture
- Source Engine multiplayer networking — Valve developer wiki
