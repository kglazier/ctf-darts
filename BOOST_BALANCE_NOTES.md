# Boost balance — deferred improvements

Notes from a 2026-04-29 session looking at why bots can outpace the
player when boosting in a straight line. The minimal fix shipped that
day was:

- **Touch-deflection normalization while boosting.** When `wants_sprint`
  is true and the joystick is past the deadzone, `move_dir` is
  normalized to length 1 before the ACCEL integration. Reason: on
  phone, hitting full deflection means dragging the thumb to the
  joystick base edge — most users sit at ~70%, which gives
  `ACCEL_eff = 1400` and drag-equilibrium ≈ 467, below the 576 boost
  cap. Bots, having no drag, always hit 576 in straight line. Patched
  in both `apply_input_to_player` (input.rs) and `client_predict_local`
  (net.rs).

If the player still feels slower than bots after that, in priority
order:

## 1. Apply player physics to bots (top-speed parity)

Today bot velocity converges via lerp toward `desired = steering *
current_max` with NO drag (`bot.rs:716-728`). Player has DRAG=3 fighting
ACCEL=2000. In curves and partial inputs, the bot retains speed better
than the player can.

Fix: in `drive_bots`, replace the steering integration with the same
ACCEL/DRAG model the player uses. Bot's `steering` (already a unit-ish
vector) becomes the bot's `move_dir`. Keep per-difficulty feel by
scaling ACCEL per difficulty (Easy = sluggish, Hard = full).

Trade: bots will turn / accelerate exactly like a player, which means
they can no longer "snap" their velocity around walls and obstacles.
Hard's chase pressure may need a small ACCEL bump to feel relentless
without breaking parity.

## 2. Match boost regen across difficulties

`sync_bot_stamina_regen` (`bot.rs:237-247`) sets bot regen per difficulty:
- Easy 0.10/s
- Medium 0.14/s
- **Hard 0.22/s**

Player default is 0.20/s (`player.rs:37`). Hard's 0.22 gives it a
slightly higher boost duty cycle than the player. Either delete the
system entirely (all ships use the 0.20 default) or set all values to
0.20. Difficulty can still vary via WHEN bots boost (urgency gating in
`bot.rs:589-621`), not how fast they refill.

## 3. Joystick-curve remap (alternative to #1's normalize-while-boosting)

If full-thrust-on-boost feels too binary, replace it with a softer
remap: `strength = (move_dir.length() * 2.0).min(1.0)`. Past 50%
deflection you get full thrust; below 50% it ramps linearly. Preserves
analog precision at low deflection without requiring perfect input to
hit max speed.

## Reference numbers

- `ACCEL = 2000`, `DRAG = 3` (`input.rs:342-343`, `net.rs:1131-1132`)
- `MaxSpeed(320)` for all ships (`player.rs:201`)
- `SPRINT_MUL = 1.8` (`SPRINT_MUL_CARRY = 1.4` while carrying)
- Boost cap = `320 * 1.8 = 576`
- Player drag-equilibrium at full deflection = `2000/3 = 666` → capped at 576 ✓
- Player drag-equilibrium at 0.7 deflection = `1400/3 = 467` → below 576 ✗
- `SPRINT_DRAIN = 0.7/s` (same for player and bots)
- Bot stamina hysteresis: start thresholds 1.1/0.85/0.7 (Easy/Med/Hard),
  drain to 0 once started (`bot.rs:597-606`)
