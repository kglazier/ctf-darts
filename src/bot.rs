use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::arena::{Base, Wall};
use crate::flag::{Flag, FlagState};
use crate::movement::{MaxSpeed, Velocity};
use crate::player::{PlayerControlled, Ship, Stamina, Thrusting, SHIP_RADIUS};
use crate::tag::Respawning;
use crate::team::Team;
use crate::GameSet;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum BotRole {
    Grabber,
    Guardian,
}

#[derive(Component)]
pub struct AllyBot;

#[derive(Component, Clone, Copy)]
pub struct BotNumber(pub u32);

/// World-space number rendered over a bot ship.
#[derive(Component)]
pub struct BotNumberLabel;

#[derive(Component)]
pub struct LabelFor(pub Entity);

#[derive(Component, Clone, Copy)]
pub struct LabelOffset(pub Vec2);


#[derive(Resource, Default)]
pub struct PlayerActivity {
    pub defender_bias: f32,
}

/// Per-bot difficulty. Tap the bot to cycle.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum BotDifficulty {
    Easy,
    Medium,
    Hard,
}

impl Default for BotDifficulty {
    fn default() -> Self {
        BotDifficulty::Medium
    }
}

impl BotDifficulty {
    pub fn next(self) -> Self {
        match self {
            BotDifficulty::Easy => BotDifficulty::Medium,
            BotDifficulty::Medium => BotDifficulty::Hard,
            BotDifficulty::Hard => BotDifficulty::Easy,
        }
    }
}

impl BotDifficulty {
    pub fn label(self) -> &'static str {
        match self {
            BotDifficulty::Easy => "EASY",
            BotDifficulty::Medium => "MEDIUM",
            BotDifficulty::Hard => "HARD",
        }
    }
}

/// Per-ally mode. Tap the ally ship to cycle.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AllyMode {
    #[default]
    Auto,
    Offense,
    Defense,
}

impl AllyMode {
    pub fn label(self) -> &'static str {
        match self {
            AllyMode::Auto => "AUTO",
            AllyMode::Offense => "OFFENSE",
            AllyMode::Defense => "DEFENSE",
        }
    }
    pub fn next(self) -> Self {
        match self {
            AllyMode::Auto => AllyMode::Offense,
            AllyMode::Offense => AllyMode::Defense,
            AllyMode::Defense => AllyMode::Auto,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Urgency {
    Low,
    Med,
    High,
}

pub struct BotPlugin;
impl Plugin for BotPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerActivity>()
            .add_systems(
                Update,
                (
                    observe_player,
                    assign_ally_role,
                    assign_enemy_roles,
                    sync_bot_stamina_regen,
                    drive_bots,
                )
                    .chain()
                    .in_set(GameSet::Ai),
            )
            .add_systems(Update, update_bot_number_labels.in_set(GameSet::Hud));
    }
}


fn observe_player(
    time: Res<Time>,
    mut activity: ResMut<PlayerActivity>,
    player: Query<(&Transform, &Ship, Option<&Respawning>), With<PlayerControlled>>,
    bases: Query<(&Transform, &Base)>,
) {
    let dt = time.delta_seconds();
    let Ok((ptf, pship, respawning)) = player.get_single() else { return };
    // Freeze the EMA while the player is dead/respawning — otherwise the
    // ally reads the stale or teleporting position and flips its role.
    if respawning.is_some() {
        return;
    }
    let Some(own_base) = bases
        .iter()
        .find(|(_, b)| b.team == pship.team)
        .map(|(tf, _)| tf.translation.truncate())
    else {
        return;
    };
    let near = ptf.translation.truncate().distance(own_base) < 300.0;
    let target: f32 = if near { 1.0 } else { 0.0 };
    // Longer time-constant (3.5s) so short dips near/far from base don't
    // flip role mid-scramble.
    let alpha = (dt / 3.5).min(1.0);
    activity.defender_bias = activity.defender_bias * (1.0 - alpha) + target * alpha;
}

/// Assign roles among enemy bots. Whichever live bot is closest to its own
/// base takes Guardian; the rest take Grabber. But: if an opposing ship is
/// already threatening the team's base/flag, don't reshuffle — the Guardian
/// stays Guardian even when their teammate dies, so the base isn't left open.
fn assign_enemy_roles(
    bases: Query<(&Transform, &Base)>,
    opposing: Query<(&Transform, &Ship), Without<Respawning>>,
    mut enemies: Query<
        (Entity, &Transform, &mut BotRole, &Ship, Option<&Respawning>),
        (With<BotDifficulty>, Without<AllyBot>),
    >,
) {
    use std::collections::HashMap;

    // Snapshot per bot: (entity, dist-to-own-base, team, respawning).
    let mut rows: Vec<(Entity, f32, Team, bool)> = Vec::new();
    for (e, tf, _, ship, respawning) in enemies.iter() {
        let Some((btf, _)) = bases.iter().find(|(_, b)| b.team == ship.team) else {
            continue;
        };
        let base_pos = btf.translation.truncate();
        let dist = tf.translation.truncate().distance(base_pos);
        rows.push((e, dist, ship.team, respawning.is_some()));
    }

    let mut by_team: HashMap<Team, Vec<(Entity, f32, bool)>> = HashMap::new();
    for (e, d, t, r) in rows {
        by_team.entry(t).or_default().push((e, d, r));
    }

    const THREAT_RADIUS: f32 = 500.0;
    for (team, group) in by_team {
        let Some((btf, _)) = bases.iter().find(|(_, b)| b.team == team) else {
            continue;
        };
        let base_pos = btf.translation.truncate();

        // Is an opposing ship already near our base?
        let threat_present = opposing.iter().any(|(tf, ship)| {
            ship.team != team
                && tf.translation.truncate().distance(base_pos) < THREAT_RADIUS
        });

        let mut live: Vec<(Entity, f32)> = group
            .iter()
            .filter(|(_, _, r)| !*r)
            .map(|(e, d, _)| (*e, *d))
            .collect();
        if live.is_empty() {
            continue;
        }
        live.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        // Force a Guardian whenever:
        //   - A threat is near our base (even if both bots were on offense),
        //   - or we have 2+ live bots (normal positional assignment).
        // With 1 live bot and no threat, let them press offense.
        let guardian = if threat_present || live.len() >= 2 {
            Some(live[0].0)
        } else {
            None
        };

        for (e, _, _) in &group {
            if let Ok((_, _, mut role, _, _)) = enemies.get_mut(*e) {
                let desired = if Some(*e) == guardian {
                    BotRole::Guardian
                } else {
                    BotRole::Grabber
                };
                if *role != desired {
                    *role = desired;
                }
            }
        }
    }
}

/// Difficulty-scaled stamina regen — Hard refills faster, so its duty cycle
/// on boost is higher, which gives it a real average-speed advantage over
/// Medium. Player's regen is untouched (different component path).
fn sync_bot_stamina_regen(
    mut q: Query<(&mut crate::player::Stamina, &BotDifficulty)>,
) {
    for (mut s, d) in &mut q {
        s.regen = match d {
            BotDifficulty::Easy => 0.10,
            BotDifficulty::Medium => 0.14,
            BotDifficulty::Hard => 0.22,
        };
    }
}

fn assign_ally_role(
    activity: Res<PlayerActivity>,
    mut q: Query<(&mut BotRole, &AllyMode), With<AllyBot>>,
) {
    for (mut role, mode) in &mut q {
        let desired = match *mode {
            AllyMode::Offense => BotRole::Grabber,
            AllyMode::Defense => BotRole::Guardian,
            AllyMode::Auto => {
                // Wider hysteresis so the role doesn't flicker between
                // offense and defense in mid-fight.
                if activity.defender_bias > 0.7 {
                    BotRole::Grabber
                } else if activity.defender_bias < 0.2 {
                    BotRole::Guardian
                } else {
                    continue;
                }
            }
        };
        if *role != desired {
            *role = desired;
        }
    }
}

#[derive(Clone, Copy)]
struct ShipInfo {
    entity: Entity,
    pos: Vec2,
    vel: Vec2,
    team: Team,
    thrusting: bool,
}

/// Naive lead-the-target: aim for where the target will be in ~dist/speed seconds
/// assuming it keeps its current velocity. Caps the lead time so we don't
/// overshoot when the target is on a curved path.
fn intercept_point(pursuer_pos: Vec2, target_pos: Vec2, target_vel: Vec2, pursuer_speed: f32) -> Vec2 {
    let dist = target_pos.distance(pursuer_pos);
    let t = (dist / pursuer_speed).clamp(0.0, 1.2);
    target_pos + target_vel * t
}

/// When our flag is stolen, decide if *this* bot should chase the thief or
/// trust a teammate. Defensive roles always chase. Offensive roles only
/// chase when they're clearly the better intercept (100-unit lead buffer).
fn own_state_chase_candidate(
    own_state: FlagState,
    self_entity: Entity,
    team: Team,
    role: BotRole,
    self_carries: bool,
    ships_snap: &[ShipInfo],
    pos: Vec2,
) -> Option<(ShipInfo, bool)> {
    let FlagState::Carried(thief) = own_state else { return None };
    let thief_info = ships_snap.iter().find(|s| s.entity == thief).copied()?;
    let my_dist = thief_info.pos.distance(pos);
    let teammate_best = ships_snap
        .iter()
        .filter(|s| s.entity != self_entity && s.entity != thief && s.team == team)
        .map(|s| s.pos.distance(thief_info.pos))
        .fold(f32::INFINITY, f32::min);
    // If WE'RE carrying and our flag is also stolen, mutual annihilation
    // (carrier-vs-carrier) is the only path to recovery — no one else on
    // our team can do anything useful while neither flag is at base. So
    // ALWAYS chase in that situation, regardless of role or distance.
    // Otherwise, normal: Guardians always chase, Grabbers only if clearly
    // best positioned (100px buffer to avoid two bots both rushing).
    let chase = self_carries
        || matches!(role, BotRole::Guardian)
        || my_dist + 100.0 < teammate_best;
    Some((thief_info, chase))
}

/// Orbit-evade: wobbling circle around own base, rotation direction chosen
/// to move away from nearest threat, clamped into the arena interior.
fn orbit_evade_target(
    pos: Vec2,
    own_base_pos: Vec2,
    self_entity: Entity,
    team: Team,
    t_now: f32,
    ships_snap: &[ShipInfo],
) -> (Option<Vec2>, Urgency) {
    let to_center = (-own_base_pos).normalize_or_zero();
    let orbit_center = own_base_pos + to_center * 200.0;
    let radius_wobble = (t_now * 1.3 + self_entity.index() as f32 * 1.7).sin() * 55.0;
    let orbit_radius = 220.0 + radius_wobble;

    let rel = pos - orbit_center;
    let bot_angle = if rel.length_squared() < 1.0 {
        0.0
    } else {
        rel.y.atan2(rel.x)
    };

    let nearest_enemy = ships_snap
        .iter()
        .filter(|s| s.team != team)
        .min_by(|a, b| {
            a.pos
                .distance(pos)
                .partial_cmp(&b.pos.distance(pos))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    let lead_sign = if let Some(threat) = nearest_enemy {
        let trel = threat.pos - orbit_center;
        let threat_angle = trel.y.atan2(trel.x);
        if (bot_angle - threat_angle).sin() > 0.0 { 1.0 } else { -1.0 }
    } else {
        1.0
    };

    let lead_wobble = (t_now * 1.9 + self_entity.index() as f32 * 0.9).sin() * 0.4;
    let lead = (1.2 + lead_wobble) * lead_sign;
    let target_angle = bot_angle + lead;
    let target = orbit_center + Vec2::new(target_angle.cos(), target_angle.sin()) * orbit_radius;

    let bx = crate::ARENA_WIDTH * 0.5 - 120.0;
    let by = crate::ARENA_HEIGHT * 0.5 - 100.0;
    let clamped = Vec2::new(target.x.clamp(-bx, bx), target.y.clamp(-by, by));
    (Some(clamped), Urgency::High)
}

fn drive_bots(
    time: Res<Time>,
    mode: Res<crate::projectile::GameMode>,
    mut bots: Query<
        (
            Entity,
            &Transform,
            &Ship,
            &BotRole,
            &BotDifficulty,
            &mut Velocity,
            &MaxSpeed,
            &mut Stamina,
            &mut Thrusting,
            Option<&crate::flag::CarryingFlag>,
            Option<&Respawning>,
        ),
        Without<PlayerControlled>,
    >,
    player_ships: Query<
        (Entity, &Transform, &Ship, &Thrusting, &Velocity),
        (With<PlayerControlled>, Without<Respawning>),
    >,
    flags: Query<(&Transform, &Flag)>,
    bases: Query<(&Transform, &Base)>,
    walls: Query<(&Transform, &Wall), Without<Velocity>>,
) {
    let dt = time.delta_seconds();
    let t_now = time.elapsed_seconds();

    // Snapshot all ships first, via the immutable view of `bots` + a separate
    // player-only query — avoiding a mut/immut aliasing conflict on Thrusting.
    let mut ships_snap: Vec<ShipInfo> = bots
        .iter()
        .filter_map(|(e, tf, s, _, _, vel, _, _, thrusting, _, respawning)| {
            if respawning.is_some() {
                None
            } else {
                Some(ShipInfo {
                    entity: e,
                    pos: tf.translation.truncate(),
                    vel: vel.0,
                    team: s.team,
                    thrusting: thrusting.0,
                })
            }
        })
        .collect();
    for (e, tf, s, thrust, vel) in &player_ships {
        ships_snap.push(ShipInfo {
            entity: e,
            pos: tf.translation.truncate(),
            vel: vel.0,
            team: s.team,
            thrusting: thrust.0,
        });
    }

    for (
        bot_entity,
        bot_tf,
        ship,
        role,
        difficulty,
        mut vel,
        max_speed,
        mut stamina,
        mut thrusting,
        carrying,
        respawning,
    ) in &mut bots
    {
        if respawning.is_some() {
            vel.0 = Vec2::ZERO;
            thrusting.0 = false;
            continue;
        }

        let pos = bot_tf.translation.truncate();

        let (own_flag_pos, own_state) = flags
            .iter()
            .find(|(_, f)| f.team == ship.team)
            .map(|(tf, f)| (tf.translation.truncate(), f.state))
            .unwrap_or((pos, FlagState::Home));
        let (enemy_flag_pos, enemy_state) = flags
            .iter()
            .find(|(_, f)| f.team == ship.team.opposite())
            .map(|(tf, f)| (tf.translation.truncate(), f.state))
            .unwrap_or((pos, FlagState::Home));

        let own_base_pos = bases
            .iter()
            .find(|(_, b)| b.team == ship.team)
            .map(|(tf, _)| tf.translation.truncate())
            .unwrap_or(own_flag_pos);
        let enemy_base_pos = bases
            .iter()
            .find(|(_, b)| b.team == ship.team.opposite())
            .map(|(tf, _)| tf.translation.truncate())
            .unwrap_or(enemy_flag_pos);

        let (raw_target, urgency): (Option<Vec2>, Urgency) = if carrying.is_some() {
            // Can't score while our flag is in enemy hands. Instead of sitting
            // on base waiting to get tagged, evade: flee the nearest threat
            // while biasing toward our own side.
            let can_score = !matches!(own_state, FlagState::Carried(_));
            if can_score {
                // If our flag is dropped in the field, swing by it first to
                // touch-return. Ensures the flag is safely home so it can't
                // be re-grabbed while we're running the enemy flag back.
                let target = if matches!(own_state, FlagState::Dropped(_)) {
                    own_flag_pos
                } else {
                    own_base_pos
                };
                (Some(target), Urgency::High)
            } else if let Some((thief_info, chase_it)) = own_state_chase_candidate(
                own_state,
                bot_entity,
                ship.team,
                *role,
                /*self_carries=*/ true,
                &ships_snap,
                pos,
            ) {
                if chase_it {
                    // We can't score with our flag stolen anyway — help the
                    // team recover by intercepting the thief. Tagging them
                    // drops our flag (ally can return it), and we still hold
                    // the enemy flag for a quick capture afterward.
                    let aim = intercept_point(pos, thief_info.pos, thief_info.vel, 480.0);
                    (Some(aim), Urgency::High)
                } else {
                    // Teammate is a better intercept — stay evasive.
                    orbit_evade_target(
                        pos,
                        own_base_pos,
                        bot_entity,
                        ship.team,
                        t_now,
                        &ships_snap,
                    )
                }
            } else {
                // No thief (e.g. our flag is Dropped). Keep orbiting to stay
                // alive while someone retrieves it.
                orbit_evade_target(
                    pos,
                    own_base_pos,
                    bot_entity,
                    ship.team,
                    t_now,
                    &ships_snap,
                )
            }
        } else {
            match role {
                BotRole::Grabber => grabber_target(
                    pos,
                    ship.team,
                    bot_entity,
                    own_state,
                    enemy_state,
                    enemy_flag_pos,
                    own_base_pos,
                    &ships_snap,
                ),
                BotRole::Guardian => guardian_target(
                    pos,
                    ship.team,
                    bot_entity,
                    own_state,
                    own_flag_pos,
                    own_base_pos,
                    enemy_base_pos,
                    &ships_snap,
                ),
            }
        };

        let Some(raw_tgt) = raw_target else {
            vel.0 *= 1.0 - (2.0 * dt).min(1.0);
            thrusting.0 = false;
            continue;
        };

        let tgt = route_around_walls(pos, raw_tgt, &walls, SHIP_RADIUS + 6.0);

        // On-path kill: is an enemy sitting roughly on the line we're already
        // driving? If so, we can tag them without deviating — just boost.
        let on_path_kill = {
            let path_dir = (tgt - pos).normalize_or_zero();
            let mut hit = false;
            if path_dir.length_squared() > 0.5 {
                for s in &ships_snap {
                    if s.team == ship.team {
                        continue;
                    }
                    let rel = s.pos - pos;
                    let forward_dist = rel.dot(path_dir);
                    if forward_dist < 30.0 || forward_dist > 260.0 {
                        continue;
                    }
                    let lateral = (rel - path_dir * forward_dist).length();
                    if lateral < 55.0 {
                        hit = true;
                        break;
                    }
                }
            }
            hit
        };

        // Boost gating — on-path kills are a Hard-only skill (Easy/Medium
        // should not surgically boost-intercept). In Shooter mode nobody
        // boosts; the button fires projectiles instead.
        let was_thrusting = thrusting.0;
        // Hysteresis: starting a fresh boost burst requires substantially
        // recharged stamina, but an in-progress burst can drain to empty.
        // Without this, bots flick boost on the instant stamina crosses the
        // low-water mark, producing a strobe that no human can match.
        let start_min = match *difficulty {
            BotDifficulty::Easy => 1.1,
            BotDifficulty::Medium => 0.85,
            BotDifficulty::Hard => 0.7,
        };
        let stamina_ok = if was_thrusting {
            stamina.current > 0.0
        } else {
            stamina.current >= start_min
        };
        let raw_want = if on_path_kill && *difficulty == BotDifficulty::Hard {
            true
        } else {
            match (*difficulty, urgency) {
                (BotDifficulty::Easy, _) => false,
                (BotDifficulty::Medium, Urgency::High) => true,
                (BotDifficulty::Medium, _) => false,
                (BotDifficulty::Hard, Urgency::High) => true,
                (BotDifficulty::Hard, Urgency::Med) => was_thrusting,
                (BotDifficulty::Hard, Urgency::Low) => false,
            }
        };
        let want_boost = *mode == crate::projectile::GameMode::Classic
            && raw_want
            && stamina_ok;
        thrusting.0 = want_boost;
        if thrusting.0 {
            stamina.current = (stamina.current - dt * 0.7).max(0.0);
            if stamina.current <= 0.0 {
                thrusting.0 = false;
            }
        }
        let sprint_mul: f32 = if carrying.is_some() { 1.4 } else { 1.8 };
        let current_max = if thrusting.0 {
            max_speed.0 * sprint_mul
        } else {
            max_speed.0
        };

        let diff = tgt - pos;
        let dist = diff.length();
        if dist < 2.0 {
            vel.0 *= 1.0 - (3.0 * dt).min(1.0);
            continue;
        }
        let dir = diff / dist;

        // Reactive wall repulsion
        const LOOK_AHEAD: f32 = 90.0;
        let mut avoid = Vec2::ZERO;
        for (wall_tf, wall) in walls.iter() {
            let wall_center = wall_tf.translation.truncate();
            let wd = pos - wall_center;
            let clamped = Vec2::new(
                wd.x.clamp(-wall.half_extents.x, wall.half_extents.x),
                wd.y.clamp(-wall.half_extents.y, wall.half_extents.y),
            );
            let closest = wall_center + clamped;
            let offset = pos - closest;
            let odist = offset.length();
            if odist > 0.01 && odist < LOOK_AHEAD {
                let strength = 1.0 - odist / LOOK_AHEAD;
                avoid += (offset / odist) * strength * strength;
            }
        }

        // Dodge nearby enemies. Carriers dodge any enemy (carrier-exception
        // in the tag rule); non-carriers only dodge boosters. Evasion skill
        // scales with difficulty so Medium isn't untouchable with the flag.
        let (dodge_range, dodge_weight): (f32, f32) = if carrying.is_some() {
            match *difficulty {
                BotDifficulty::Easy => (140.0, 0.3),
                BotDifficulty::Medium => (190.0, 0.6),
                BotDifficulty::Hard => (215.0, 0.8),
            }
        } else {
            (210.0, 1.0)
        };
        for s in &ships_snap {
            if s.team == ship.team {
                continue;
            }
            let is_threat = carrying.is_some() || s.thrusting;
            if !is_threat {
                continue;
            }
            let off = pos - s.pos;
            let d = off.length();
            if d > 0.01 && d < dodge_range {
                let strength = 1.0 - d / dodge_range;
                avoid += (off / d) * strength * strength * dodge_weight;
            }
        }

        // Gentle teammate separation — bots should not body-check each other.
        const TEAMMATE_SPACING: f32 = 80.0;
        for s in &ships_snap {
            if s.team != ship.team || s.entity == bot_entity {
                continue;
            }
            let off = pos - s.pos;
            let d = off.length();
            if d > 0.01 && d < TEAMMATE_SPACING {
                let strength = 1.0 - d / TEAMMATE_SPACING;
                avoid += (off / d) * strength * 0.7;
            }
        }

        let wobble = if avoid.length_squared() < 0.04 && dist > 150.0 {
            let phase = t_now * 2.5 + bot_entity.index() as f32 * 1.3;
            Vec2::new(-dir.y, dir.x) * phase.sin() * 0.12
        } else {
            Vec2::ZERO
        };

        // Cap avoid magnitude so wall+enemy repulsion can't fully cancel the
        // direction vector (which would zero out steering and stall the bot
        // any time the target ends up near a wall).
        let avoid_capped = avoid.clamp_length_max(0.85);
        let steering = (dir + avoid_capped + wobble).normalize_or_zero();
        let desired = steering * current_max;
        // Easy bots turn sluggishly — more human reaction delay.
        let steer_gain: f32 = match *difficulty {
            BotDifficulty::Easy => 2.0,
            BotDifficulty::Medium => 2.8,
            BotDifficulty::Hard => 3.5,
        };
        let steer = (desired - vel.0) * steer_gain * dt;
        vel.0 += steer;
        if vel.0.length() > current_max {
            vel.0 = vel.0.normalize() * current_max;
        }
    }
}

fn grabber_target(
    pos: Vec2,
    team: Team,
    self_entity: Entity,
    own_state: FlagState,
    enemy_state: FlagState,
    enemy_flag_pos: Vec2,
    own_base_pos: Vec2,
    ships_snap: &[ShipInfo],
) -> (Option<Vec2>, Urgency) {
    // If a teammate is carrying the enemy flag, `enemy_flag_pos` is literally
    // the teammate's position — driving toward it rams them. Pivot goals to
    // protect our flag so the teammate can score.
    let teammate_carries_enemy = matches!(
        enemy_state,
        FlagState::Carried(c) if ships_snap.iter().any(|s| s.entity == c && s.team == team)
    );
    if teammate_carries_enemy {
        match own_state {
            FlagState::Carried(thief) => {
                if let Some(ti) = ships_snap.iter().find(|s| s.entity == thief) {
                    let aim = intercept_point(pos, ti.pos, ti.vel, 480.0);
                    return (Some(aim), Urgency::High);
                }
            }
            FlagState::Dropped(p) => return (Some(p), Urgency::Med),
            FlagState::Home => {
                return (Some(own_base_pos), Urgency::Low);
            }
        }
    }

    if matches!(enemy_state, FlagState::Dropped(_)) {
        return (Some(enemy_flag_pos), Urgency::Med);
    }

    if let FlagState::Carried(thief) = own_state {
        if let Some(thief_info) = ships_snap.iter().find(|s| s.entity == thief) {
            let thief_pos = thief_info.pos;
            let my_dist = thief_pos.distance(pos);
            let teammate_best = ships_snap
                .iter()
                .filter(|s| s.entity != self_entity && s.entity != thief && s.team == team)
                .map(|s| s.pos.distance(thief_pos))
                .fold(f32::INFINITY, f32::min);
            if my_dist + 100.0 < teammate_best {
                let aim = intercept_point(pos, thief_pos, thief_info.vel, 480.0);
                return (Some(aim), Urgency::High);
            }
        }
        return (Some(enemy_flag_pos), Urgency::Med);
    }

    (Some(enemy_flag_pos), Urgency::Low)
}

fn guardian_target(
    pos: Vec2,
    team: Team,
    self_entity: Entity,
    own_state: FlagState,
    own_flag_pos: Vec2,
    own_base_pos: Vec2,
    enemy_base_pos: Vec2,
    ships_snap: &[ShipInfo],
) -> (Option<Vec2>, Urgency) {
    if let FlagState::Carried(thief) = own_state {
        if let Some(thief_info) = ships_snap.iter().find(|s| s.entity == thief) {
            let aim = intercept_point(pos, thief_info.pos, thief_info.vel, 480.0);
            return (Some(aim), Urgency::High);
        }
    }
    if matches!(own_state, FlagState::Dropped(_)) {
        return (Some(own_flag_pos), Urgency::Med);
    }
    let threat = ships_snap
        .iter()
        .filter(|s| s.entity != self_entity && s.team != team)
        .filter(|s| s.pos.distance(own_base_pos) < 500.0)
        .min_by(|a, b| {
            a.pos
                .distance(pos)
                .partial_cmp(&b.pos.distance(pos))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    if let Some(t) = threat {
        let aim = intercept_point(pos, t.pos, t.vel, 480.0);
        return (Some(aim), Urgency::Med);
    }
    let toward = (enemy_base_pos - own_flag_pos).normalize_or_zero();
    (Some(own_flag_pos + toward * 120.0), Urgency::Low)
}

fn route_around_walls(
    bot_pos: Vec2,
    target_pos: Vec2,
    walls: &Query<(&Transform, &Wall), Without<Velocity>>,
    margin: f32,
) -> Vec2 {
    for (wall_tf, wall) in walls.iter() {
        let center = wall_tf.translation.truncate();
        let half = wall.half_extents + Vec2::splat(margin);
        if !segment_intersects_aabb(bot_pos, target_pos, center, half) {
            continue;
        }
        let corners = [
            center + Vec2::new(half.x, half.y),
            center + Vec2::new(half.x, -half.y),
            center + Vec2::new(-half.x, half.y),
            center + Vec2::new(-half.x, -half.y),
        ];
        let best = corners
            .iter()
            .min_by(|a, b| {
                let da = bot_pos.distance(**a) + a.distance(target_pos);
                let db = bot_pos.distance(**b) + b.distance(target_pos);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
            .unwrap_or(target_pos);
        return best;
    }
    target_pos
}

fn segment_intersects_aabb(p0: Vec2, p1: Vec2, center: Vec2, half: Vec2) -> bool {
    let d = p1 - p0;
    let mut tmin: f32 = 0.0;
    let mut tmax: f32 = 1.0;

    if d.x.abs() < 1e-6 {
        if p0.x < center.x - half.x || p0.x > center.x + half.x {
            return false;
        }
    } else {
        let inv = 1.0 / d.x;
        let mut t1 = (center.x - half.x - p0.x) * inv;
        let mut t2 = (center.x + half.x - p0.x) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return false;
        }
    }

    if d.y.abs() < 1e-6 {
        if p0.y < center.y - half.y || p0.y > center.y + half.y {
            return false;
        }
    } else {
        let inv = 1.0 / d.y;
        let mut t1 = (center.y - half.y - p0.y) * inv;
        let mut t2 = (center.y + half.y - p0.y) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return false;
        }
    }

    true
}

/// Keep each number label planted on its ship, upright, hidden when the
/// ship itself is hidden. We mirror the SHIP's Visibility (not the
/// Respawning component) because online clients don't get a Respawning
/// component on remote ships — they replicate visibility from snapshot
/// instead. Reading ship visibility works on host AND client uniformly.
fn update_bot_number_labels(
    ships: Query<(&Transform, &Visibility), Without<BotNumberLabel>>,
    mut labels: Query<
        (&mut Transform, &mut Visibility, &LabelFor, Option<&LabelOffset>),
        With<BotNumberLabel>,
    >,
) {
    for (mut ltf, mut vis, target, offset) in &mut labels {
        let Ok((ship_tf, ship_vis)) = ships.get(target.0) else {
            continue;
        };
        let off = offset.map(|o| o.0).unwrap_or(Vec2::ZERO);
        let z = ltf.translation.z;
        ltf.translation = (ship_tf.translation.truncate() + off).extend(z);
        ltf.rotation = Quat::IDENTITY;
        *vis = match ship_vis {
            Visibility::Hidden => Visibility::Hidden,
            _ => Visibility::Visible,
        };
    }
}
