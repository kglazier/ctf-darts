use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use crate::bot::BotDifficulty;
use crate::flag::Score;
use crate::hud::MatchState;
use crate::GameSet;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Menu,
    /// Online lobby — host/join, peer list, bot slot picker. S1 ships a
    /// "Coming Soon" stub; S2 wires up matchbox + lobby UI here.
    Lobby,
    Playing,
    GameOver,
    /// Transient state: cleans up and auto-transitions back to Playing so
    /// OnEnter(Playing) re-runs the spawn chain.
    Restarting,
}

/// Resource-gated pause — leaves entities alive but freezes gameplay.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub struct Paused(pub bool);

/// Local player's team for the active match. Captured during spawn so
/// `spawn_gameover` can decide "you won / you lost" without depending on
/// any ship entity (those are despawned by OnExit(Playing) before
/// OnEnter(GameOver) runs).
#[derive(Resource, Clone, Copy)]
pub struct LocalTeam(pub crate::team::Team);
impl Default for LocalTeam {
    fn default() -> Self { Self(crate::team::Team::Red) }
}

/// Endless mode has no score target — play as long as you want.
/// Derived from `ScoreTarget` at match start; used by the HUD to expose
/// bot +/- controls and all per-bot rows.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub struct EndlessMode(pub bool);

/// Menu-selected winning score. `None` = unlimited (freeplay).
#[derive(Resource, Clone, Copy)]
pub struct ScoreTarget(pub Option<u32>);
impl Default for ScoreTarget {
    fn default() -> Self { Self(Some(5)) }
}

/// Per-team ship slot config. Humans are real players (1 local in solo, plus
/// remote peers in online); bots fill the remaining slots. Solo defaults to
/// 1 red human + bots; online sets `red_humans`/`blue_humans` from the
/// lobby's connected peers and `*_bots` from the lobby's bot picker.
///
/// Renamed-in-place from the old "BotCounts" so the Endless-mode HUD +/-
/// controls and OnEnter reset path keep working unchanged.
#[derive(Resource, Clone, Copy)]
pub struct BotCounts {
    pub red_allies: u32,
    pub blue_enemies: u32,
    pub red_humans: u32,
    pub blue_humans: u32,
    /// Difficulty applied to red allies. In solo this is identical to
    /// blue_difficulty (both come from SelectedDifficulty). In online the
    /// host picks each side independently in the lobby.
    pub red_difficulty: BotDifficulty,
    pub blue_difficulty: BotDifficulty,
}
impl Default for BotCounts {
    fn default() -> Self {
        Self {
            red_allies: 1,
            blue_enemies: 2,
            red_humans: 1,
            blue_humans: 0,
            red_difficulty: BotDifficulty::Medium,
            blue_difficulty: BotDifficulty::Medium,
        }
    }
}

/// Run-condition used by the gameplay SystemSets.
pub fn playing_unpaused(state: Res<State<AppState>>, pause: Res<Paused>) -> bool {
    matches!(state.get(), AppState::Playing) && !pause.0
}

/// Entities spawned for an active match. Despawned on OnExit(Playing).
#[derive(Component)]
pub struct PlayingEntity;

/// Entities that live during Menu only.
#[derive(Component)]
pub struct MenuEntity;

/// Marker for entities owned by the Lobby state UI; `cleanup::<LobbyEntity>`
/// despawns them OnExit(Lobby). Defined here so other modules don't have to
/// import `lobby` just to attach the marker.
#[derive(Component)]
pub struct LobbyEntity;

/// Entities that live during GameOver only.
#[derive(Component)]
pub struct GameOverEntity;

#[derive(Resource, Clone, Copy, Debug)]
pub struct SelectedLevel(pub usize);
impl Default for SelectedLevel {
    fn default() -> Self { Self(0) }
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SelectedDifficulty(pub BotDifficulty);

#[derive(Resource, Serialize, Deserialize, Debug, Default)]
pub struct Progress {
    pub best: HashMap<usize, BotDifficulty>,
}

impl Progress {
    pub fn record(&mut self, level: usize, diff: BotDifficulty) {
        let better = match self.best.get(&level).copied() {
            None => true,
            Some(BotDifficulty::Easy) => !matches!(diff, BotDifficulty::Easy),
            Some(BotDifficulty::Medium) => matches!(diff, BotDifficulty::Hard),
            Some(BotDifficulty::Hard) => false,
        };
        if better {
            self.best.insert(level, diff);
            save_progress(self);
        }
    }
    pub fn label(&self, level: usize) -> &'static str {
        match self.best.get(&level) {
            Some(BotDifficulty::Easy) => "EASY",
            Some(BotDifficulty::Medium) => "MEDIUM",
            Some(BotDifficulty::Hard) => "HARD",
            None => "-",
        }
    }
}

/// Level definition — wall layout plus a display name.
#[derive(Clone, Copy)]
pub struct WallSpec {
    pub x: f32,
    pub y: f32,
    pub half_w: f32,
    pub half_h: f32,
}

pub struct LevelDef {
    pub name: &'static str,
    pub walls: &'static [WallSpec],
}

pub const LEVELS: [LevelDef; 9] = [
    LevelDef {
        name: "Open Field",
        walls: &[],
    },
    LevelDef {
        name: "Central Divide",
        walls: &[WallSpec { x: 0.0, y: 0.0, half_w: 10.0, half_h: 200.0 }],
    },
    LevelDef {
        name: "Crossroads",
        walls: &[
            WallSpec { x: 0.0, y: 200.0, half_w: 220.0, half_h: 10.0 },
            WallSpec { x: 0.0, y: -200.0, half_w: 220.0, half_h: 10.0 },
            WallSpec { x: -340.0, y: 0.0, half_w: 10.0, half_h: 130.0 },
            WallSpec { x: 340.0, y: 0.0, half_w: 10.0, half_h: 130.0 },
        ],
    },
    LevelDef {
        name: "Four Pillars",
        walls: &[
            WallSpec { x: -260.0, y: 150.0, half_w: 40.0, half_h: 40.0 },
            WallSpec { x: 260.0, y: 150.0, half_w: 40.0, half_h: 40.0 },
            WallSpec { x: -260.0, y: -150.0, half_w: 40.0, half_h: 40.0 },
            WallSpec { x: 260.0, y: -150.0, half_w: 40.0, half_h: 40.0 },
            WallSpec { x: 0.0, y: 0.0, half_w: 120.0, half_h: 10.0 },
        ],
    },
    LevelDef {
        name: "Gauntlet",
        walls: &[
            // Shorter staggered walls with clear gaps so bot routing doesn't
            // deadlock on overlapping wall-avoidance fields.
            WallSpec { x: -160.0, y: 180.0, half_w: 150.0, half_h: 10.0 },
            WallSpec { x: 160.0, y: -180.0, half_w: 150.0, half_h: 10.0 },
            WallSpec { x: 0.0, y: 0.0, half_w: 10.0, half_h: 110.0 },
        ],
    },
    LevelDef {
        name: "Three Lanes",
        walls: &[
            // Two long horizontals split the arena into three corridors.
            // No center vertical, so cross-lane jukes are still possible
            // around the wall ends.
            WallSpec { x: 0.0, y: 130.0, half_w: 320.0, half_h: 10.0 },
            WallSpec { x: 0.0, y: -130.0, half_w: 320.0, half_h: 10.0 },
        ],
    },
    LevelDef {
        name: "Plus",
        walls: &[
            // Center plus — forces commits to a side. Arms are short enough
            // that bots can route around; longer arms deadlocked the AI.
            WallSpec { x: 0.0, y: 0.0, half_w: 110.0, half_h: 10.0 },
            WallSpec { x: 0.0, y: 0.0, half_w: 10.0, half_h: 110.0 },
        ],
    },
    LevelDef {
        name: "Zigzag",
        walls: &[
            // Staggered horizontals create a snaking path through center.
            WallSpec { x: -200.0, y: 110.0, half_w: 130.0, half_h: 10.0 },
            WallSpec { x: 200.0, y: 0.0, half_w: 130.0, half_h: 10.0 },
            WallSpec { x: -200.0, y: -110.0, half_w: 130.0, half_h: 10.0 },
        ],
    },
    LevelDef {
        name: "Bottleneck",
        walls: &[
            // Top + bottom verticals funnel traffic through center; two
            // small mid pillars discourage straight-line dashes.
            WallSpec { x: 0.0, y: 230.0, half_w: 10.0, half_h: 90.0 },
            WallSpec { x: 0.0, y: -230.0, half_w: 10.0, half_h: 90.0 },
            WallSpec { x: -140.0, y: 0.0, half_w: 30.0, half_h: 30.0 },
            WallSpec { x: 140.0, y: 0.0, half_w: 30.0, half_h: 30.0 },
        ],
    },
];

pub struct GamePlugin;
impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>()
            .init_resource::<SelectedLevel>()
            .init_resource::<SelectedDifficulty>()
            .init_resource::<Paused>()
            .init_resource::<EndlessMode>()
            .init_resource::<ScoreTarget>()
            .init_resource::<BotCounts>()
            .init_resource::<ExitConfirmOpen>()
            .init_resource::<LocalTeam>()
            .insert_resource(load_progress())
            .add_systems(Startup, (setup_camera, on_startup))
            .add_systems(OnEnter(AppState::Menu), spawn_menu)
            .add_systems(OnExit(AppState::Menu), cleanup::<MenuEntity>)
            .add_systems(
                OnEnter(AppState::Playing),
                (
                    reset_online_state_for_solo,
                    enter_playing,
                    crate::arena::spawn_arena,
                    crate::player::spawn_ships,
                    crate::input::spawn_joystick_ui,
                    crate::hud::setup_hud,
                    spawn_pause_button,
                    crate::flag::spawn_flags,
                    crate::hud::setup_bot_hud,
                )
                    .chain(),
            )
            .add_systems(OnExit(AppState::Playing), (cleanup::<PlayingEntity>, unpause))
            .add_systems(OnEnter(AppState::GameOver), spawn_gameover)
            .add_systems(OnExit(AppState::GameOver), (cleanup::<GameOverEntity>, clear_restart_countdown))
            .add_systems(
                Update,
                (
                    receive_restart_event,
                    tick_restart_countdown,
                    update_countdown_overlay,
                )
                    .run_if(in_state(AppState::GameOver)),
            )
            .add_systems(OnEnter(AppState::Restarting), restart_tick)
            .add_systems(
                Update,
                detect_match_end
                    .in_set(GameSet::Hud)
                    .run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                (menu_button_clicks, update_menu_level_display)
                    .run_if(in_state(AppState::Menu)),
            )
            .add_systems(
                Update,
                (gameover_button_clicks).run_if(in_state(AppState::GameOver)),
            )
            .add_systems(
                Update,
                (
                    pause_button_click,
                    pause_overlay_sync,
                    pause_overlay_clicks,
                    exit_confirm_overlay_sync,
                    exit_confirm_clicks,
                )
                    .run_if(in_state(AppState::Playing)),
            )
            // Pause whenever the app loses window focus (Android home/recents,
            // notification shade pull-down, incoming call). Without this the
            // game keeps simulating in the background, draining battery and
            // — in online — sending stale snapshots while the player isn't
            // looking. We don't auto-unpause on focus return; player must tap
            // RESUME so they aren't surprised by an instant tag.
            .add_systems(Update, pause_on_focus_loss);
    }
}

fn pause_on_focus_loss(
    mut events: EventReader<bevy::window::WindowFocused>,
    mut pause: ResMut<Paused>,
    state: Res<State<AppState>>,
    net_mode: Res<crate::net::NetworkMode>,
) {
    // Online matches can't pause — the other peers keep simulating, so
    // pausing one side just desyncs it. Skip the auto-pause entirely in
    // online mode (host or client) and let the match keep running while
    // the window is in the background.
    if *net_mode != crate::net::NetworkMode::Solo {
        return;
    }
    for ev in events.read() {
        if !ev.focused && matches!(state.get(), AppState::Playing) {
            pause.0 = true;
        }
    }
}

fn setup_camera(mut commands: Commands) {
    use bevy::render::camera::ScalingMode;
    let mut bundle = Camera2dBundle::default();
    bundle.projection.scaling_mode = ScalingMode::AutoMin {
        min_width: crate::ARENA_WIDTH + 80.0,
        min_height: crate::ARENA_HEIGHT + 80.0,
    };
    commands.spawn(bundle);
}

fn on_startup() {
    // Could run first-time setup here; currently no-op.
}

fn cleanup<T: Component>(mut commands: Commands, q: Query<Entity, With<T>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

#[allow(clippy::too_many_arguments)]
fn enter_playing(
    mut match_state: ResMut<MatchState>,
    mut score: ResMut<Score>,
    mut pause: ResMut<Paused>,
    target: Res<ScoreTarget>,
    diff: Res<SelectedDifficulty>,
    mut endless: ResMut<EndlessMode>,
    mut counts: ResMut<BotCounts>,
    net_mode: Res<crate::net::NetworkMode>,
    config: Res<crate::net::OnlineMatchConfig>,
    mut snapshots: ResMut<crate::net::SnapshotBuffer>,
    mut clock: ResMut<crate::net::ClientClock>,
) {
    // Clear any leftover snapshots from a previous match. Otherwise on a
    // restart the client briefly applies the old snapshot (with the
    // winning score), `detect_match_end` fires again, and the player gets
    // bounced back to GameOver before the new match can start.
    snapshots.recent.clear();
    // Reset client-side time calibration so the first snapshot of the
    // new match recalibrates fresh (host time may have advanced
    // significantly during the GameOver/countdown).
    clock.offset = None;
    *score = Score::default();
    *match_state = MatchState::default();
    match target.0 {
        Some(n) => {
            match_state.target_score = n;
            endless.0 = false;
        }
        None => {
            match_state.target_score = u32::MAX;
            endless.0 = true;
        }
    }
    pause.0 = false;
    if *net_mode != crate::net::NetworkMode::Solo {
        // Online: BotCounts comes from the lobby's OnlineMatchConfig
        // (which both peers have). Don't reset to defaults — that would
        // wipe humans/bots/difficulty the host just picked.
        counts.red_humans = config.red_humans;
        counts.blue_humans = config.blue_humans;
        counts.red_allies = config.red_bots;
        counts.blue_enemies = config.blue_bots;
        counts.red_difficulty = config.red_difficulty;
        counts.blue_difficulty = config.blue_difficulty;
    } else {
        // Solo: a non-endless match resets bot counts to defaults so the
        // HUD's +/- experiment doesn't bleed across replays. Difficulty
        // mirrors the player's level pick and is uniform across teams.
        if !endless.0 {
            *counts = BotCounts::default();
        }
        counts.red_difficulty = diff.0;
        counts.blue_difficulty = diff.0;
    }
}

/// Reset online-only state when entering Solo. Without this, leaving an
/// online match (Menu / exit-confirm) leaves stale `LocalNetId` /
/// `PeerSlots` / `HostPeerId` in place; the next solo match spawns
/// using whatever slot the user was in online — possibly producing a
/// playerless game (LocalNetId points at a NetId that's now a bot in
/// the solo layout).
fn reset_online_state_for_solo(
    net_mode: Res<crate::net::NetworkMode>,
    mut local_net_id: ResMut<crate::net::LocalNetId>,
    mut peer_slots: ResMut<crate::net::PeerSlots>,
    mut host_peer: ResMut<crate::net::HostPeerId>,
) {
    if *net_mode != crate::net::NetworkMode::Solo {
        return;
    }
    *local_net_id = crate::net::LocalNetId(1);
    peer_slots.0.clear();
    host_peer.0 = None;
}

fn unpause(mut pause: ResMut<Paused>) {
    pause.0 = false;
}

fn restart_tick(mut next: ResMut<NextState<AppState>>) {
    next.set(AppState::Playing);
}

#[derive(Component)]
struct PauseButton;
#[derive(Component)]
struct PauseOverlay;
#[derive(Component)]
struct PauseRestart;
#[derive(Component)]
struct PauseExit;
#[derive(Component)]
struct PauseResume;

// Online-only exit-confirmation overlay components.
#[derive(Component)]
struct ExitConfirmOverlay;
#[derive(Component)]
struct ExitConfirmYes;
#[derive(Component)]
struct ExitConfirmNo;
#[derive(Resource, Default)]
struct ExitConfirmOpen(bool);

fn spawn_pause_button(
    mut commands: Commands,
    net_mode: Res<crate::net::NetworkMode>,
) {
    // In solo, the button pauses (II). In online, you can't actually
    // pause a multiplayer match — the button becomes an X that exits
    // the match (with confirmation) instead.
    let online = *net_mode != crate::net::NetworkMode::Solo;
    let label = if online { "X" } else { "||" };
    let bg = if online {
        Color::srgba(0.4, 0.15, 0.15, 0.85)
    } else {
        Color::srgba(0.1, 0.1, 0.22, 0.85)
    };
    commands.spawn((
        ButtonBundle {
            style: Style {
                position_type: PositionType::Absolute,
                top: Val::Px(20.0),
                left: Val::Percent(50.0),
                margin: UiRect {
                    left: Val::Px(-28.0),
                    top: Val::Px(52.0),
                    ..default()
                },
                width: Val::Px(56.0),
                height: Val::Px(30.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            background_color: bg.into(),
            border_radius: BorderRadius::all(Val::Px(5.0)),
            focus_policy: FocusPolicy::Block,
            ..default()
        },
        PauseButton,
        PlayingEntity,
    ))
    .with_children(|b| {
        b.spawn(TextBundle::from_section(
            label,
            TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
        ));
    });
}

fn pause_button_click(
    q: Query<&Interaction, (Changed<Interaction>, With<PauseButton>)>,
    net_mode: Res<crate::net::NetworkMode>,
    mut pause: ResMut<Paused>,
    mut exit_confirm: ResMut<ExitConfirmOpen>,
) {
    for i in &q {
        if *i != Interaction::Pressed {
            continue;
        }
        if *net_mode == crate::net::NetworkMode::Solo {
            pause.0 = !pause.0;
        } else {
            // Online — open the exit-confirmation overlay instead of
            // pausing. Pausing only on one peer would just desync that
            // peer from the live match.
            exit_confirm.0 = true;
        }
    }
}

fn pause_overlay_sync(
    pause: Res<Paused>,
    existing: Query<Entity, With<PauseOverlay>>,
    mut commands: Commands,
) {
    if !pause.is_changed() {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn_recursive();
    }
    if !pause.0 {
        return;
    }
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(20.0),
                    ..default()
                },
                background_color: Color::srgba(0.0, 0.0, 0.0, 0.75).into(),
                z_index: ZIndex::Global(100),
                ..default()
            },
            PauseOverlay,
            PlayingEntity,
        ))
        .with_children(|root| {
            root.spawn(TextBundle::from_section(
                "PAUSED",
                TextStyle { font_size: 56.0, color: Color::WHITE, ..default() },
            ));
            root.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(20.0),
                    ..default()
                },
                ..default()
            })
            .with_children(|row| {
                // RESUME — closes the overlay and unpauses the game.
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(140.0),
                            height: Val::Px(48.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.25, 0.5, 0.3, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    PauseResume,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "RESUME",
                        TextStyle { font_size: 20.0, color: Color::WHITE, ..default() },
                    ));
                });
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(140.0),
                            height: Val::Px(48.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.2, 0.35, 0.6, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    PauseRestart,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "RESTART",
                        TextStyle { font_size: 20.0, color: Color::WHITE, ..default() },
                    ));
                });
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(140.0),
                            height: Val::Px(48.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.35, 0.2, 0.5, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    PauseExit,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "EXIT",
                        TextStyle { font_size: 20.0, color: Color::WHITE, ..default() },
                    ));
                });
            });
        });
}

fn pause_overlay_clicks(
    restart: Query<&Interaction, (Changed<Interaction>, With<PauseRestart>)>,
    exit: Query<&Interaction, (Changed<Interaction>, With<PauseExit>)>,
    resume: Query<&Interaction, (Changed<Interaction>, With<PauseResume>)>,
    mut pause: ResMut<Paused>,
    mut next: ResMut<NextState<AppState>>,
) {
    for i in &resume {
        if *i == Interaction::Pressed {
            pause.0 = false;
        }
    }
    for i in &restart {
        if *i == Interaction::Pressed {
            next.set(AppState::Restarting);
        }
    }
    for i in &exit {
        if *i == Interaction::Pressed {
            next.set(AppState::Menu);
        }
    }
}

fn exit_confirm_overlay_sync(
    open: Res<ExitConfirmOpen>,
    existing: Query<Entity, With<ExitConfirmOverlay>>,
    mut commands: Commands,
) {
    if !open.is_changed() {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn_recursive();
    }
    if !open.0 {
        return;
    }
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(16.0),
                    ..default()
                },
                background_color: Color::srgba(0.0, 0.0, 0.0, 0.75).into(),
                z_index: ZIndex::Global(100),
                ..default()
            },
            ExitConfirmOverlay,
            PlayingEntity,
        ))
        .with_children(|root| {
            root.spawn(TextBundle::from_section(
                "Leave the match?",
                TextStyle { font_size: 30.0, color: Color::WHITE, ..default() },
            ));
            root.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(16.0),
                    ..default()
                },
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(120.0),
                            height: Val::Px(48.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.55, 0.2, 0.2, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    ExitConfirmYes,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "LEAVE",
                        TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
                    ));
                });
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(120.0),
                            height: Val::Px(48.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.2, 0.4, 0.25, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    ExitConfirmNo,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "STAY",
                        TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
                    ));
                });
            });
        });
}

fn exit_confirm_clicks(
    yes_q: Query<&Interaction, (Changed<Interaction>, With<ExitConfirmYes>)>,
    no_q: Query<&Interaction, (Changed<Interaction>, With<ExitConfirmNo>)>,
    mut commands: Commands,
    mut open: ResMut<ExitConfirmOpen>,
    mut net_mode: ResMut<crate::net::NetworkMode>,
    mut next: ResMut<NextState<AppState>>,
) {
    for i in &no_q {
        if *i == Interaction::Pressed {
            open.0 = false;
        }
    }
    for i in &yes_q {
        if *i == Interaction::Pressed {
            // Same teardown as the GameOver MENU button: drop the socket
            // and reset NetworkMode so the online-only systems stop
            // running, then go to the menu.
            commands.remove_resource::<crate::net::NetSocket>();
            *net_mode = crate::net::NetworkMode::Solo;
            open.0 = false;
            next.set(AppState::Menu);
        }
    }
}

#[derive(Component)]
struct MenuLevelCycle;
#[derive(Component)]
struct MenuLevelNameText;
#[derive(Component)]
struct MenuLevelBestText;
#[derive(Component)]
struct MenuStartButton(BotDifficulty);

#[derive(Component)]
struct MenuOnlineButton;

#[derive(Component)]
struct MenuModeToggle;

#[derive(Component)]
struct MenuModeText;

#[derive(Component)]
struct MenuScoreDecrease;

#[derive(Component)]
struct MenuScoreIncrease;

#[derive(Component)]
struct MenuScoreUnlimited;

#[derive(Component)]
struct MenuScoreText;

fn spawn_menu(
    mut commands: Commands,
    progress: Res<Progress>,
    mode: Res<crate::projectile::GameMode>,
    target: Res<ScoreTarget>,
    selected_level: Res<SelectedLevel>,
) {
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(12.0),
                    padding: UiRect::all(Val::Px(20.0)),
                    ..default()
                },
                background_color: Color::srgba(0.02, 0.02, 0.06, 0.95).into(),
                ..default()
            },
            MenuEntity,
        ))
        .with_children(|root| {
            root.spawn(TextBundle::from_section(
                "SPACE BOOSTERS",
                TextStyle { font_size: 48.0, color: Color::srgb(1.0, 1.0, 0.4), ..default() },
            ));

            // Mode toggle
            root.spawn((
                ButtonBundle {
                    style: Style {
                        width: Val::Px(240.0),
                        height: Val::Px(36.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    background_color: Color::srgba(0.18, 0.18, 0.32, 0.95).into(),
                    border_radius: BorderRadius::all(Val::Px(5.0)),
                    focus_policy: FocusPolicy::Block,
                    ..default()
                },
                MenuModeToggle,
            ))
            .with_children(|b| {
                b.spawn((
                    TextBundle::from_section(
                        format!("MODE: {}", mode.label()),
                        TextStyle { font_size: 16.0, color: Color::WHITE, ..default() },
                    ),
                    MenuModeText,
                ));
            });

            // Play Online — opens lobby stub for now (real lobby in S2).
            root.spawn((
                ButtonBundle {
                    style: Style {
                        width: Val::Px(240.0),
                        height: Val::Px(36.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    background_color: Color::srgba(0.2, 0.4, 0.25, 0.95).into(),
                    border_radius: BorderRadius::all(Val::Px(5.0)),
                    focus_policy: FocusPolicy::Block,
                    ..default()
                },
                MenuOnlineButton,
            ))
            .with_children(|b| {
                b.spawn(TextBundle::from_section(
                    "PLAY ONLINE",
                    TextStyle { font_size: 16.0, color: Color::WHITE, ..default() },
                ));
            });

            // Target-score picker. Selecting Unlimited enables freeplay
            // behavior (no win, bot +/- controls visible in-game).
            root.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
                background_color: Color::srgba(0.12, 0.12, 0.22, 0.8).into(),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(32.0),
                            height: Val::Px(32.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.25, 0.25, 0.45, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    MenuScoreDecrease,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "-",
                        TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
                    ));
                });
                row.spawn(NodeBundle {
                    style: Style {
                        width: Val::Px(180.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    ..default()
                })
                .with_children(|c| {
                    c.spawn((
                        TextBundle::from_section(
                            format_score(target.0),
                            TextStyle { font_size: 16.0, color: Color::WHITE, ..default() },
                        ),
                        MenuScoreText,
                    ));
                });
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(32.0),
                            height: Val::Px(32.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.25, 0.25, 0.45, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    MenuScoreIncrease,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "+",
                        TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
                    ));
                });
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(120.0),
                            height: Val::Px(32.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.3, 0.2, 0.5, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    MenuScoreUnlimited,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "UNLIMITED",
                        TextStyle { font_size: 14.0, color: Color::WHITE, ..default() },
                    ));
                });
            });

            // Level cycle row: tap to advance, shows current name + best.
            let current_level_name = LEVELS
                .get(selected_level.0)
                .map(|l| l.name)
                .unwrap_or("?");
            root.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
                background_color: Color::srgba(0.08, 0.08, 0.18, 0.8).into(),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(260.0),
                            height: Val::Px(40.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.25, 0.25, 0.45, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    MenuLevelCycle,
                ))
                .with_children(|b| {
                    b.spawn((
                        TextBundle::from_section(
                            format!("Level: {}  >", current_level_name),
                            TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
                        ),
                        MenuLevelNameText,
                    ));
                });
                row.spawn((
                    TextBundle::from_section(
                        format!("Best: {}", progress.label(selected_level.0)),
                        TextStyle {
                            font_size: 14.0,
                            color: Color::srgba(1.0, 1.0, 0.5, 0.9),
                            ..default()
                        },
                    ),
                    MenuLevelBestText,
                ));
            });

            // Difficulty start row — click to start at the cycled level.
            root.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(10.0),
                    ..default()
                },
                ..default()
            })
            .with_children(|row| {
                for (lbl, diff, color) in [
                    ("EASY",   BotDifficulty::Easy,   Color::srgba(0.25, 0.45, 0.3, 0.95)),
                    ("MEDIUM", BotDifficulty::Medium, Color::srgba(0.25, 0.35, 0.5, 0.95)),
                    ("HARD",   BotDifficulty::Hard,   Color::srgba(0.5,  0.25, 0.3, 0.95)),
                ] {
                    row.spawn((
                        ButtonBundle {
                            style: Style {
                                width: Val::Px(110.0),
                                height: Val::Px(48.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            background_color: color.into(),
                            border_radius: BorderRadius::all(Val::Px(6.0)),
                            focus_policy: FocusPolicy::Block,
                            ..default()
                        },
                        MenuStartButton(diff),
                    ))
                    .with_children(|b| {
                        b.spawn(TextBundle::from_section(
                            lbl,
                            TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
                        ));
                    });
                }
            });
        });
}

pub fn format_score(target: Option<u32>) -> String {
    match target {
        Some(n) => format!("POINTS TO WIN: {}", n),
        None => "POINTS TO WIN: UNLIMITED".into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn menu_button_clicks(
    cycle_q: Query<&Interaction, (Changed<Interaction>, With<MenuLevelCycle>)>,
    start_q: Query<(&Interaction, &MenuStartButton), Changed<Interaction>>,
    mode_q: Query<&Interaction, (Changed<Interaction>, With<MenuModeToggle>)>,
    online_q: Query<&Interaction, (Changed<Interaction>, With<MenuOnlineButton>)>,
    score_dec_q: Query<&Interaction, (Changed<Interaction>, With<MenuScoreDecrease>)>,
    score_inc_q: Query<&Interaction, (Changed<Interaction>, With<MenuScoreIncrease>)>,
    score_unl_q: Query<&Interaction, (Changed<Interaction>, With<MenuScoreUnlimited>)>,
    mut level: ResMut<SelectedLevel>,
    mut diff: ResMut<SelectedDifficulty>,
    mut target: ResMut<ScoreTarget>,
    mut mode: ResMut<crate::projectile::GameMode>,
    mut mode_text: Query<&mut Text, (With<MenuModeText>, Without<MenuScoreText>)>,
    mut score_text: Query<&mut Text, With<MenuScoreText>>,
    mut next: ResMut<NextState<AppState>>,
) {
    for i in &mode_q {
        if *i == Interaction::Pressed {
            *mode = mode.next();
            for mut t in &mut mode_text {
                t.sections[0].value = format!("MODE: {}", mode.label());
            }
        }
    }
    for i in &online_q {
        if *i == Interaction::Pressed {
            next.set(AppState::Lobby);
        }
    }
    let mut score_changed = false;
    for i in &score_dec_q {
        if *i == Interaction::Pressed {
            let current = target.0.unwrap_or(5);
            target.0 = Some(current.saturating_sub(1).max(1));
            score_changed = true;
        }
    }
    for i in &score_inc_q {
        if *i == Interaction::Pressed {
            let current = target.0.unwrap_or(5);
            target.0 = Some((current + 1).min(15));
            score_changed = true;
        }
    }
    for i in &score_unl_q {
        if *i == Interaction::Pressed {
            target.0 = match target.0 {
                Some(_) => None,
                None => Some(5),
            };
            score_changed = true;
        }
    }
    if score_changed {
        for mut t in &mut score_text {
            t.sections[0].value = format_score(target.0);
        }
    }
    for i in &cycle_q {
        if *i == Interaction::Pressed {
            level.0 = (level.0 + 1) % LEVELS.len();
        }
    }
    for (i, btn) in &start_q {
        if *i == Interaction::Pressed {
            diff.0 = btn.0;
            next.set(AppState::Playing);
        }
    }
}

/// Update the level-name + best-difficulty labels on the menu when
/// SelectedLevel changes (e.g. cycle button tapped).
fn update_menu_level_display(
    selected: Res<SelectedLevel>,
    progress: Res<Progress>,
    mut name_q: Query<
        &mut Text,
        (With<MenuLevelNameText>, Without<MenuLevelBestText>),
    >,
    mut best_q: Query<&mut Text, With<MenuLevelBestText>>,
) {
    if !selected.is_changed() {
        return;
    }
    let level_name = LEVELS.get(selected.0).map(|l| l.name).unwrap_or("?");
    for mut t in &mut name_q {
        t.sections[0].value = format!("Level: {}  >", level_name);
    }
    for mut t in &mut best_q {
        t.sections[0].value = format!("Best: {}", progress.label(selected.0));
    }
}

fn detect_match_end(state: Res<MatchState>, mut next: ResMut<NextState<AppState>>) {
    if state.winner.is_some() {
        next.set(AppState::GameOver);
    }
}


#[derive(Component)]
struct GameOverRetry;
#[derive(Component)]
struct GameOverMenu;
#[derive(Component)]
struct CountdownOverlayText;
/// Default seconds before an online restart fires. Long enough for
/// players to notice the overlay and decide whether to bail to menu;
/// short enough not to feel slow. Tune via the LobbyEvent::Restart
/// payload if needed.
const RESTART_COUNTDOWN_SECS: f32 = 5.0;

#[allow(clippy::too_many_arguments)]
fn spawn_gameover(
    mut commands: Commands,
    score: Res<Score>,
    state: Res<MatchState>,
    level: Res<SelectedLevel>,
    diff: Res<SelectedDifficulty>,
    mut progress: ResMut<Progress>,
    net_mode: Res<crate::net::NetworkMode>,
    local_team: Res<LocalTeam>,
) {
    // Stash team is set in spawn_ship when the local Player is created.
    // Reading from a Resource (not a query) means we don't depend on
    // the ship still existing — it's already despawned by this point.
    let you_won = state.winner == Some(local_team.0);
    // Solo records best-difficulty progress; online matches don't update
    // the per-level progression (it's friend-vs-friend, not a campaign run).
    if you_won && *net_mode == crate::net::NetworkMode::Solo {
        progress.record(level.0, diff.0);
    }

    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(16.0),
                    ..default()
                },
                background_color: Color::srgba(0.0, 0.0, 0.0, 0.85).into(),
                ..default()
            },
            GameOverEntity,
        ))
        .with_children(|root| {
            root.spawn(TextBundle::from_section(
                if you_won { "YOU WON" } else { "YOU LOST" },
                TextStyle {
                    font_size: 56.0,
                    color: if you_won {
                        Color::srgb(0.5, 1.0, 0.5)
                    } else {
                        Color::srgb(1.0, 0.5, 0.5)
                    },
                    ..default()
                },
            ));
            root.spawn(TextBundle::from_section(
                format!("{} - {}", score.red, score.blue),
                TextStyle { font_size: 40.0, color: Color::WHITE, ..default() },
            ));

            // Online-mode hint so the host knows they can restart and the
            // client knows they're waiting on the host.
            let hint = match *net_mode {
                crate::net::NetworkMode::OnlineHost => Some(
                    "Tap PLAY AGAIN to start a new match for everyone, or MENU to leave.",
                ),
                crate::net::NetworkMode::OnlineClient => Some(
                    "Host can start a new match. You'll see a countdown when they do.",
                ),
                crate::net::NetworkMode::Solo => None,
            };
            if let Some(text) = hint {
                root.spawn(TextBundle::from_section(
                    text,
                    TextStyle {
                        font_size: 14.0,
                        color: Color::srgba(1.0, 1.0, 1.0, 0.7),
                        ..default()
                    },
                ));
            }

            // Countdown text — populated by `update_countdown_overlay`.
            // Hidden when no countdown is active (empty string renders as
            // a zero-height node in Bevy UI).
            root.spawn((
                TextBundle::from_section(
                    "",
                    TextStyle {
                        font_size: 48.0,
                        color: Color::srgb(1.0, 1.0, 0.4),
                        ..default()
                    },
                ),
                CountdownOverlayText,
            ));

            // Action buttons. Online client gets no RETRY (host owns
            // restart); they can only MENU out.
            root.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(20.0),
                    ..default()
                },
                ..default()
            })
            .with_children(|row| {
                let show_retry =
                    *net_mode != crate::net::NetworkMode::OnlineClient;
                if show_retry {
                    let label = match *net_mode {
                        crate::net::NetworkMode::OnlineHost => "PLAY AGAIN",
                        _ => "RETRY",
                    };
                    row.spawn((
                        ButtonBundle {
                            style: Style {
                                width: Val::Px(160.0),
                                height: Val::Px(48.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            background_color: Color::srgba(0.2, 0.35, 0.6, 0.95).into(),
                            border_radius: BorderRadius::all(Val::Px(6.0)),
                            focus_policy: FocusPolicy::Block,
                            ..default()
                        },
                        GameOverRetry,
                    ))
                    .with_children(|b| {
                        b.spawn(TextBundle::from_section(
                            label,
                            TextStyle { font_size: 20.0, color: Color::WHITE, ..default() },
                        ));
                    });
                } else {
                    row.spawn(TextBundle::from_section(
                        "Waiting for host…",
                        TextStyle {
                            font_size: 18.0,
                            color: Color::srgba(1.0, 1.0, 1.0, 0.7),
                            ..default()
                        },
                    ));
                }

                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(140.0),
                            height: Val::Px(48.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.35, 0.2, 0.5, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    GameOverMenu,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "MENU",
                        TextStyle { font_size: 20.0, color: Color::WHITE, ..default() },
                    ));
                });
            });
        });
}

#[allow(clippy::too_many_arguments)]
fn gameover_button_clicks(
    retry_q: Query<&Interaction, (Changed<Interaction>, With<GameOverRetry>)>,
    menu_q: Query<&Interaction, (Changed<Interaction>, With<GameOverMenu>)>,
    mut next: ResMut<NextState<AppState>>,
    mut net_mode: ResMut<crate::net::NetworkMode>,
    mut config: ResMut<crate::net::OnlineMatchConfig>,
    mut bot_counts: ResMut<BotCounts>,
    mut peer_slots: ResMut<crate::net::PeerSlots>,
    mut local_net_id: ResMut<crate::net::LocalNetId>,
    mut countdown: ResMut<crate::net::RestartCountdown>,
    mut commands: Commands,
    socket: Option<ResMut<crate::net::NetSocket>>,
    peers: Res<crate::net::ConnectedPeers>,
) {
    for i in &retry_q {
        if *i != Interaction::Pressed {
            continue;
        }
        match *net_mode {
            crate::net::NetworkMode::Solo => {
                next.set(AppState::Playing);
            }
            crate::net::NetworkMode::OnlineHost => {
                // Recompute team setup using *currently-connected* peers.
                // Anyone who dropped during GameOver is naturally absent
                // from `peers`, so their slot becomes a bot per the
                // helper's "preserve total team size" rule.
                let (new_config, assignments, slots) =
                    crate::net::compute_match_setup(&peers, &config);
                *config = new_config;
                bot_counts.red_humans = config.red_humans;
                bot_counts.blue_humans = config.blue_humans;
                bot_counts.red_allies = config.red_bots;
                bot_counts.blue_enemies = config.blue_bots;
                peer_slots.0 = slots;
                *local_net_id = crate::net::LocalNetId(1);

                if let Some(mut s) = socket {
                    crate::net::broadcast(
                        &mut s,
                        &peers,
                        &crate::net::NetMessage::Lobby(crate::net::LobbyEvent::Restart {
                            config: *config,
                            assignments,
                            countdown_secs: RESTART_COUNTDOWN_SECS,
                        }),
                    );
                }
                countdown.0 = Some(Timer::from_seconds(
                    RESTART_COUNTDOWN_SECS,
                    TimerMode::Once,
                ));
                let _ = &mut commands;
                return;
            }
            crate::net::NetworkMode::OnlineClient => {
                // Client RETRY shouldn't render, but guard anyway.
            }
        }
    }
    for i in &menu_q {
        if *i == Interaction::Pressed {
            // Leaving the match — drop the socket AND reset NetworkMode
            // back to Solo. Without the mode reset, online-only systems
            // (client_send_input, host_send_snapshot, etc.) panic because
            // they ResMut<NetSocket> and the socket no longer exists.
            commands.remove_resource::<crate::net::NetSocket>();
            *net_mode = crate::net::NetworkMode::Solo;
            countdown.0 = None;
            next.set(AppState::Menu);
        }
    }
}

/// Client-side: pick up the host's Restart broadcast, mirror the new
/// assignments locally, and start the countdown. The actual transition
/// to Playing happens in `tick_restart_countdown` when the timer expires.
#[allow(clippy::too_many_arguments)]
fn receive_restart_event(
    socket: Option<ResMut<crate::net::NetSocket>>,
    net_mode: Res<crate::net::NetworkMode>,
    mut inbox: ResMut<crate::net::LobbyInbox>,
    mut config: ResMut<crate::net::OnlineMatchConfig>,
    mut bot_counts: ResMut<BotCounts>,
    mut local_net_id: ResMut<crate::net::LocalNetId>,
    mut selected_level: ResMut<SelectedLevel>,
    mut score_target: ResMut<ScoreTarget>,
    mut countdown: ResMut<crate::net::RestartCountdown>,
) {
    if *net_mode != crate::net::NetworkMode::OnlineClient {
        inbox.restarts.clear();
        return;
    }
    let Some(mut s) = socket else {
        inbox.restarts.clear();
        return;
    };
    let my_id = s.0.id().map(|p| p.to_string()).unwrap_or_default();
    let restarts = std::mem::take(&mut inbox.restarts);
    for (c, assignments, secs) in restarts {
        *config = c;
        bot_counts.red_humans = c.red_humans;
        bot_counts.blue_humans = c.blue_humans;
        bot_counts.red_allies = c.red_bots;
        bot_counts.blue_enemies = c.blue_bots;
        bot_counts.red_difficulty = c.red_difficulty;
        bot_counts.blue_difficulty = c.blue_difficulty;
        // Host can change the level/score target between matches by
        // backing out and reselecting; pull the latest each restart.
        selected_level.0 = c.level;
        score_target.0 = c.score_target;
        if let Some(a) = assignments.iter().find(|a| a.peer_id == my_id) {
            *local_net_id = crate::net::LocalNetId(a.net_id);
        }
        countdown.0 = Some(Timer::from_seconds(secs, TimerMode::Once));
    }
}

fn tick_restart_countdown(
    time: Res<Time>,
    mut countdown: ResMut<crate::net::RestartCountdown>,
    mut next: ResMut<NextState<AppState>>,
) {
    let Some(timer) = countdown.0.as_mut() else {
        return;
    };
    timer.tick(time.delta());
    if timer.finished() {
        countdown.0 = None;
        next.set(AppState::Restarting);
    }
}

fn update_countdown_overlay(
    countdown: Res<crate::net::RestartCountdown>,
    mut text_q: Query<&mut Text, With<CountdownOverlayText>>,
    mut retry_q: Query<&mut Visibility, With<GameOverRetry>>,
) {
    let active = countdown.0.is_some();
    if let Ok(mut text) = text_q.get_single_mut() {
        text.sections[0].value = match countdown.0.as_ref() {
            Some(timer) => {
                let secs_left =
                    (timer.duration().as_secs_f32() - timer.elapsed_secs()).max(0.0);
                // Round up so the player sees "5,4,3,2,1" rather than "4,3,2,1,0".
                let display = secs_left.ceil() as u32;
                format!("STARTING IN {}", display)
            }
            None => String::new(),
        };
    }
    // Hide the RETRY button once the countdown is running — clicking it
    // again does nothing useful and just clutters the screen.
    for mut vis in &mut retry_q {
        *vis = if active { Visibility::Hidden } else { Visibility::Inherited };
    }
}

fn clear_restart_countdown(mut countdown: ResMut<crate::net::RestartCountdown>) {
    // Whether we're heading to Playing (countdown finished) or to Menu
    // (someone clicked out), the overlay shouldn't carry over.
    countdown.0 = None;
}

// ---------- Persistence ----------

/// Where progress.json lives, by platform. The desktop fallback (cwd) is
/// fine for local builds; Android's launch cwd isn't writable so we point
/// at the app's internal files dir; wasm uses localStorage entirely and
/// this path is never consulted.
#[cfg(target_os = "android")]
fn save_path() -> PathBuf {
    // Hardcoded against `[package.metadata.android] package` in Cargo.toml.
    // The dir is created by Android at install time; create_dir_all in
    // save_progress is belt + suspenders.
    PathBuf::from("/data/data/com.kglazier.spaceboosters/files/progress.json")
}

#[cfg(not(any(target_os = "android", target_arch = "wasm32")))]
fn save_path() -> PathBuf {
    PathBuf::from("progress.json")
}

#[cfg(target_arch = "wasm32")]
const WEB_STORAGE_KEY: &str = "space_boosters/progress";

#[cfg(target_arch = "wasm32")]
fn web_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_progress() -> Progress {
    match std::fs::read(save_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Progress::default(),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn load_progress() -> Progress {
    web_storage()
        .and_then(|s| s.get_item(WEB_STORAGE_KEY).ok().flatten())
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_progress(progress: &Progress) {
    let Ok(bytes) = serde_json::to_vec_pretty(progress) else { return };
    let path = save_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, bytes);
}

#[cfg(target_arch = "wasm32")]
pub fn save_progress(progress: &Progress) {
    let Ok(json) = serde_json::to_string(progress) else { return };
    if let Some(s) = web_storage() {
        let _ = s.set_item(WEB_STORAGE_KEY, &json);
    }
}
