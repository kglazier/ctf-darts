use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};

use crate::bot::{
    AllyBot, AllyMode, BotDifficulty, BotNumber, BotNumberLabel, BotRole, LabelFor, LabelOffset,
};
use crate::projectile::FireCooldown;
use crate::movement::{MaxSpeed, Velocity};
use crate::physics::Collider;
use crate::tag::{RespawnPoint, Tagable};
use crate::team::Team;
use crate::GameSet;

pub const SHIP_SIZE: f32 = 22.0;
pub const SHIP_RADIUS: f32 = 14.0;

#[derive(Component)]
pub struct Ship {
    pub team: Team,
}

#[derive(Component)]
pub struct PlayerControlled;

#[derive(Component)]
pub struct Facing(pub f32);

#[derive(Component)]
pub struct Stamina {
    pub current: f32,
    pub max: f32,
    pub regen: f32,
}

impl Default for Stamina {
    fn default() -> Self {
        Self { current: 1.0, max: 1.0, regen: 0.2 }
    }
}

#[derive(Component, Default)]
pub struct Thrusting(pub bool);

pub struct PlayerPlugin;
impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, regen_stamina.in_set(GameSet::Gameplay));
    }
}

pub fn spawn_ships(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    counts: Res<crate::game::BotCounts>,
    local_net_id: Res<crate::net::LocalNetId>,
) {
    // Per-team difficulty lives in BotCounts (populated in enter_playing
    // from either solo's SelectedDifficulty or the online config).
    let red_diff = counts.red_difficulty;
    let blue_diff = counts.blue_difficulty;
    let local_id = local_net_id.0;
    let ship_mesh = meshes.add(Triangle2d::new(
        Vec2::new(SHIP_SIZE, 0.0),
        Vec2::new(-SHIP_SIZE * 0.6, SHIP_SIZE * 0.6),
        Vec2::new(-SHIP_SIZE * 0.6, -SHIP_SIZE * 0.6),
    ));

    // `number` is reused as the visible bot label. `net_id` is the stable
    // cross-peer identity for replication (matches between host and client
    // because spawn_ships runs deterministically on both sides given the
    // same MatchConfig).
    let mut number: u32 = 1;
    let mut net_id: u32 = 1;
    bevy::log::info!(
        "spawn_ships: red_humans={}, red_bots={}, blue_humans={}, blue_bots={}, LocalNetId={}",
        counts.red_humans, counts.red_allies, counts.blue_humans, counts.blue_enemies, local_id
    );

    // ---- Red team: humans first, then bot allies. ----
    // Whichever slot's net_id matches LocalNetId becomes the local Player
    // on this peer; the rest are RemoteHumans (driven by network input on
    // the host, by snapshots on other clients). Bot label numbers only
    // increment for non-Player slots.
    for h in 0..counts.red_humans {
        let pos = Vec2::new(-600.0, (h as f32) * 60.0);
        let is_local = net_id == local_id;
        let kind = if is_local {
            ShipKind::Player
        } else {
            ShipKind::RemoteHuman(number)
        };
        spawn_ship(&mut commands, &mut materials, ship_mesh.clone(), Team::Red, pos, kind, red_diff, net_id);
        if !is_local {
            number += 1;
        }
        net_id += 1;
    }
    for i in 0..counts.red_allies {
        let y = 60.0 + (counts.red_humans as f32 + i as f32) * 60.0;
        spawn_ship(
            &mut commands,
            &mut materials,
            ship_mesh.clone(),
            Team::Red,
            Vec2::new(-600.0, y),
            ShipKind::Ally(BotRole::Guardian, number),
            red_diff,
            net_id,
        );
        number += 1;
        net_id += 1;
    }

    // ---- Blue team: humans first, then bot enemies. ----
    for h in 0..counts.blue_humans {
        let y = (h as f32 - (counts.blue_humans as f32 - 1.0) * 0.5) * 80.0;
        let is_local = net_id == local_id;
        let kind = if is_local {
            ShipKind::Player
        } else {
            ShipKind::RemoteHuman(number)
        };
        spawn_ship(
            &mut commands,
            &mut materials,
            ship_mesh.clone(),
            Team::Blue,
            Vec2::new(600.0, y),
            kind,
            blue_diff,
            net_id,
        );
        if !is_local {
            number += 1;
        }
        net_id += 1;
    }
    for i in 0..counts.blue_enemies {
        // Stagger bot enemies below the human row so they don't overlap.
        let base = if counts.blue_humans > 0 {
            120.0 + (i as f32) * 80.0
        } else {
            (i as f32 - (counts.blue_enemies as f32 - 1.0) * 0.5) * 80.0
        };
        let role = if i % 2 == 0 { BotRole::Grabber } else { BotRole::Guardian };
        spawn_ship(
            &mut commands,
            &mut materials,
            ship_mesh.clone(),
            Team::Blue,
            Vec2::new(600.0, base),
            ShipKind::Enemy(role, number),
            blue_diff,
            net_id,
        );
        number += 1;
        net_id += 1;
    }
}

pub enum ShipKind {
    Player,
    /// A remote human player. Spawns as a placeholder with no driver
    /// locally — host writes into its PlayerInput from network packets;
    /// other clients see it via snapshot replication.
    RemoteHuman(u32),
    Ally(BotRole, u32),
    Enemy(BotRole, u32),
}

pub fn spawn_ship(
    commands: &mut Commands,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    mesh: Handle<Mesh>,
    team: Team,
    pos: Vec2,
    kind: ShipKind,
    difficulty: BotDifficulty,
    net_id: u32,
) {
    if matches!(kind, ShipKind::Player) {
        // Stash the local team so GameOver can show the right "you won/
        // lost" message — by the time spawn_gameover runs, the ship is
        // already despawned (OnExit(Playing) cleanup), so we can't read
        // the team from the entity.
        commands.insert_resource(crate::game::LocalTeam(team));
    }
    let facing = if matches!(team, Team::Red) { 0.0 } else { std::f32::consts::PI };
    let ship_entity = {
        let mut e = commands.spawn((
            MaterialMesh2dBundle {
                mesh: Mesh2dHandle(mesh),
                material: materials.add(team.color()),
                transform: Transform::from_translation(pos.extend(5.0))
                    .with_rotation(Quat::from_rotation_z(facing)),
                ..default()
            },
            Ship { team },
            Velocity(Vec2::ZERO),
            MaxSpeed(320.0),
            Facing(facing),
            Stamina::default(),
            Thrusting(false),
            Collider::Circle { radius: SHIP_RADIUS },
            Tagable,
            RespawnPoint(pos),
            FireCooldown::default(),
            crate::net::NetId(net_id),
            crate::game::PlayingEntity,
        ));
        let bot_number = match kind {
            ShipKind::Player => {
                e.insert((
                    PlayerControlled,
                    crate::input::PlayerInput::default(),
                    crate::net::HostInputDelayBuf::default(),
                ));
                None
            }
            ShipKind::RemoteHuman(n) => {
                // Placeholder for an online peer's ship. Carries PlayerInput
                // so S2 can write into it from network packets without a
                // schema change. No PlayerControlled marker — local input
                // and HUD won't drive it.
                e.insert((crate::input::PlayerInput::default(), BotNumber(n)));
                Some(n)
            }
            ShipKind::Ally(role, n) => {
                e.insert((role, AllyBot, AllyMode::default(), difficulty, BotNumber(n)));
                Some(n)
            }
            ShipKind::Enemy(role, n) => {
                e.insert((role, difficulty, BotNumber(n)));
                Some(n)
            }
        };
        let _ = bot_number;
        e.id()
    };

    let n = match kind {
        ShipKind::Player => return,
        ShipKind::RemoteHuman(n)
        | ShipKind::Ally(_, n)
        | ShipKind::Enemy(_, n) => n,
    };

    // Dark shadow under + bright foreground for fake-bold outline effect.
    commands.spawn((
        Text2dBundle {
            text: Text::from_section(
                format!("{}", n),
                TextStyle { font_size: 42.0, color: Color::srgba(0.0, 0.0, 0.0, 0.95), ..default() },
            ),
            transform: Transform::from_translation((pos + Vec2::new(2.0, -2.0)).extend(9.5)),
            ..default()
        },
        BotNumberLabel,
        LabelFor(ship_entity),
        LabelOffset(Vec2::new(2.0, -2.0)),
        crate::game::PlayingEntity,
    ));
    commands.spawn((
        Text2dBundle {
            text: Text::from_section(
                format!("{}", n),
                TextStyle { font_size: 42.0, color: Color::srgb(1.0, 1.0, 0.4), ..default() },
            ),
            transform: Transform::from_translation(pos.extend(10.0)),
            ..default()
        },
        BotNumberLabel,
        LabelFor(ship_entity),
        crate::game::PlayingEntity,
    ));
}

fn regen_stamina(time: Res<Time>, mut q: Query<(&mut Stamina, &Thrusting)>) {
    for (mut s, thrusting) in &mut q {
        if !thrusting.0 {
            s.current = (s.current + s.regen * time.delta_seconds()).min(s.max);
        }
    }
}

/// Build the standard triangle ship mesh. Used by spawn_ships at match
/// start AND by mid-match `add_bot` so we don't have to thread the mesh
/// handle through unrelated code.
pub fn build_ship_mesh(meshes: &mut Assets<Mesh>) -> Handle<Mesh> {
    meshes.add(Triangle2d::new(
        Vec2::new(SHIP_SIZE, 0.0),
        Vec2::new(-SHIP_SIZE * 0.6, SHIP_SIZE * 0.6),
        Vec2::new(-SHIP_SIZE * 0.6, -SHIP_SIZE * 0.6),
    ))
}

/// Spawn a single bot ship into a live match. Picks a NetId of
/// `desired_net_id` if provided (used by online clients to mirror what
/// the host chose), otherwise allocates max-existing+1. Returns the
/// chosen NetId so the host can broadcast it.
pub fn add_bot(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    team: Team,
    difficulty: BotDifficulty,
    existing_ships: &Query<(&Ship, &crate::net::NetId, Option<&BotNumber>)>,
    desired_net_id: Option<u32>,
) -> u32 {
    // Allocate NetId. On host: pick max+1. On client mirroring host: use
    // the host's choice via desired_net_id (avoids race where peers pick
    // different NetIds for the same logical bot).
    let net_id = desired_net_id.unwrap_or_else(|| {
        existing_ships
            .iter()
            .map(|(_, n, _)| n.0)
            .max()
            .unwrap_or(0)
            + 1
    });
    // Bot label number: max+1 across existing labeled ships.
    let bot_number = existing_ships
        .iter()
        .filter_map(|(_, _, n)| n.map(|b| b.0))
        .max()
        .unwrap_or(0)
        + 1;

    // Spawn near base, offset by an unused y slot so bots don't stack
    // on top of each other. Count current bots on this team for offset.
    let count_on_team = existing_ships.iter().filter(|(s, _, _)| s.team == team).count() as f32;
    let x_base = if matches!(team, Team::Red) { -600.0 } else { 600.0 };
    let pos = Vec2::new(x_base, count_on_team * 50.0 - 120.0);

    let mesh = build_ship_mesh(meshes);
    let kind = match team {
        Team::Red => ShipKind::Ally(BotRole::Guardian, bot_number),
        Team::Blue => ShipKind::Enemy(BotRole::Grabber, bot_number),
    };
    spawn_ship(commands, materials, mesh, team, pos, kind, difficulty, net_id);
    net_id
}

/// Despawn the highest-NetId bot on a team, plus its label entities.
/// Returns the NetId removed (so the host can broadcast which one to
/// drop). Returns None if the team has no removable bots.
pub fn remove_last_bot(
    commands: &mut Commands,
    team: Team,
    bots: &Query<(Entity, &Ship, &crate::net::NetId), With<BotDifficulty>>,
    labels: &Query<(Entity, &LabelFor), With<BotNumberLabel>>,
) -> Option<u32> {
    let target = bots
        .iter()
        .filter(|(_, s, _)| s.team == team)
        .max_by_key(|(_, _, n)| n.0)?;
    let (entity, _, net_id) = target;
    commands.entity(entity).despawn();
    for (label_e, label_for) in labels.iter() {
        if label_for.0 == entity {
            commands.entity(label_e).despawn();
        }
    }
    Some(net_id.0)
}
