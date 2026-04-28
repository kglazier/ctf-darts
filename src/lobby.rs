//! Online lobby UI — host a code, join a code, pick bot slots, start match.
//!
//! Flow:
//!   Choose ──Host──▶ Hosting (show code, peer list, bot picker, START)
//!         ╰─Join──▶ Joining (4-letter A-Z picker) ──Connect──▶ Joined
//!                                                              (waiting for host)
//!
//! Once the host clicks START, it broadcasts `LobbyEvent::Start { config }`
//! and transitions everyone (host + connected clients) to AppState::Playing
//! with NetworkMode::OnlineHost / OnlineClient set and OnlineMatchConfig
//! mirrored on each peer.

use bevy::prelude::*;
use bevy::ui::FocusPolicy;

use crate::game::{AppState, LobbyEntity};
use crate::net::{
    broadcast, compute_match_setup, is_blue_for_index, open_socket, sorted_peer_ids,
    ConnectedPeers, HostPeerId, LobbyEvent, LobbyInbox, LobbyRole, LocalNetId, NetMessage,
    NetSocket, NetworkMode, OnlineMatchConfig, PeerPings, PeerSlots, RosterEntry,
};

pub struct LobbyPlugin;
impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LobbyView>()
            .init_resource::<JoinCodeBuffer>()
            .init_resource::<HostLobbyCode>()
            .init_resource::<CurrentRoster>()
            .add_systems(OnEnter(AppState::Lobby), enter_lobby)
            .add_systems(OnExit(AppState::Lobby), exit_lobby)
            .add_systems(
                Update,
                (
                    handle_choose_buttons,
                    handle_text_input,
                    handle_bot_picker,
                    handle_difficulty_picker,
                    handle_level_picker,
                    handle_score_picker,
                    handle_connect_button,
                    handle_start_button,
                    handle_back_button,
                    receive_start_from_host,
                    receive_roster,
                    host_broadcast_roster,
                )
                    .run_if(in_state(AppState::Lobby)),
            )
            // Repaint runs after click handlers so a state-mutating click
            // produces a redraw on the same frame.
            .add_systems(
                Update,
                (repaint_on_view_change, rebuild_peer_list)
                    .after(host_broadcast_roster)
                    .run_if(in_state(AppState::Lobby)),
            );
    }
}

/// Currently-displayed roster on this peer. Host writes its own; client
/// writes from received `LobbyEvent::Roster`. Drives the team display.
#[derive(Resource, Default, Clone)]
struct CurrentRoster(Vec<RosterEntry>);

#[derive(Resource, Clone, Copy, PartialEq, Eq, Default, Debug)]
enum LobbyView {
    #[default]
    Choose,
    Hosting,
    Joining,
    Joined,
}

#[derive(Resource, Default)]
struct JoinCodeBuffer(String);

#[derive(Resource, Default)]
struct HostLobbyCode(String);

// ---------- Component markers ----------

#[derive(Component)]
struct HostBtn;
#[derive(Component)]
struct JoinBtn;
#[derive(Component)]
struct BackBtn;
#[derive(Component)]
struct LetterBtn(char);
#[derive(Component)]
struct BackspaceBtn;
#[derive(Component)]
struct ConnectBtn;
#[derive(Component)]
struct StartBtn;
#[derive(Component)]
struct BotAdjBtn {
    team: BotTeam,
    delta: i32,
}
/// Per-team difficulty picker — replaces the old single-difficulty row.
#[derive(Component)]
struct TeamDifficultyBtn {
    team: BotTeam,
    diff: crate::bot::BotDifficulty,
}
#[derive(Component)]
struct LevelCycleBtn;
#[derive(Component)]
struct ScoreAdjBtn(i32);
#[derive(Component)]
struct ScoreUnlimitedBtn;
#[derive(Clone, Copy, PartialEq, Eq)]
enum BotTeam {
    Red,
    Blue,
}
#[derive(Component)]
struct CodeText;
#[derive(Component)]
#[allow(dead_code)] // S4 will use the team to update the count text in place.
struct BotCountText(BotTeam);

// ---------- Lifecycle ----------

fn enter_lobby(
    mut commands: Commands,
    mut view: ResMut<LobbyView>,
    mut buf: ResMut<JoinCodeBuffer>,
    mut roster: ResMut<CurrentRoster>,
) {
    *view = LobbyView::Choose;
    buf.0.clear();
    roster.0.clear();
    let cfg = OnlineMatchConfig::default();
    paint(
        &mut commands,
        *view,
        LobbyRole::None,
        "",
        "",
        &cfg,
        &[],
        None,
        cfg.level,
        cfg.score_target,
    );
}

fn exit_lobby(
    mut commands: Commands,
    existing: Query<Entity, With<LobbyEntity>>,
) {
    // Despawn the lobby UI on the way out — otherwise the menu sits on
    // top of the gameplay view after START. Socket lifecycle (keep on
    // Start, drop on Back) is owned by the per-button handlers, not by
    // OnExit, so we don't touch NetSocket here.
    for e in &existing {
        commands.entity(e).despawn_recursive();
    }
}

// ---------- Painting ----------
//
// Rather than diff the UI tree, we despawn and re-spawn LobbyEntity on each
// view change. This is O(small) and keeps the code linear; lobbies aren't
// performance-critical.

#[allow(clippy::too_many_arguments)]
fn repaint_on_view_change(
    mut commands: Commands,
    view: Res<LobbyView>,
    buf: Res<JoinCodeBuffer>,
    code: Res<HostLobbyCode>,
    config: Res<OnlineMatchConfig>,
    roster: Res<CurrentRoster>,
    pings: Res<PeerPings>,
    role: Res<LobbyRole>,
    selected_level: Res<crate::game::SelectedLevel>,
    score_target: Res<crate::game::ScoreTarget>,
    existing: Query<Entity, With<LobbyEntity>>,
) {
    if !view.is_changed()
        && !buf.is_changed()
        && !code.is_changed()
        && !config.is_changed()
        && !roster.is_changed()
        && !pings.is_changed()
        && !selected_level.is_changed()
        && !score_target.is_changed()
    {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn_recursive();
    }
    let worst_ping = pings.0.values().copied().max();
    paint(
        &mut commands,
        *view,
        *role,
        &code.0,
        &buf.0,
        &config,
        &roster.0,
        worst_ping,
        selected_level.0,
        score_target.0,
    );
}

/// No-op now — `repaint_on_view_change` rewrites the whole tree on every
/// state change including roster, so peer-list text doesn't need its own
/// in-place updater. Kept as an empty system to avoid a Plugin churn.
fn rebuild_peer_list() {}

#[allow(clippy::too_many_arguments)]
fn paint(
    commands: &mut Commands,
    view: LobbyView,
    role: LobbyRole,
    host_code: &str,
    join_buf: &str,
    config: &OnlineMatchConfig,
    roster: &[RosterEntry],
    worst_ping_ms: Option<u32>,
    selected_level: usize,
    score_target: Option<u32>,
) {
    let root_node = NodeBundle {
        style: Style {
            position_type: PositionType::Absolute,
            top: Val::Px(0.0),
            left: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: Val::Px(8.0),
            padding: UiRect::all(Val::Px(20.0)),
            ..default()
        },
        background_color: Color::srgba(0.02, 0.02, 0.06, 0.95).into(),
        ..default()
    };

    commands
        .spawn((root_node, LobbyEntity))
        .with_children(|root| {
            root.spawn(TextBundle::from_section(
                "ONLINE LOBBY",
                TextStyle {
                    font_size: 24.0,
                    color: Color::srgb(0.7, 1.0, 0.7),
                    ..default()
                },
            ));

            match view {
                LobbyView::Choose => paint_choose(root),
                LobbyView::Hosting => paint_hosting(
                    root,
                    host_code,
                    config,
                    roster,
                    worst_ping_ms,
                    selected_level,
                    score_target,
                ),
                LobbyView::Joining => paint_joining(root, join_buf),
                LobbyView::Joined => paint_joined(root, role, roster, worst_ping_ms),
            }

            // Universal Back button.
            spawn_button(
                root,
                BackBtn,
                "BACK",
                Color::srgba(0.35, 0.2, 0.5, 0.95),
                160.0,
                42.0,
            );
        });
}

fn paint_choose(root: &mut ChildBuilder) {
    root.spawn(TextBundle::from_section(
        "Host a new game or join a friend's code.",
        TextStyle {
            font_size: 16.0,
            color: Color::srgba(1.0, 1.0, 1.0, 0.75),
            ..default()
        },
    ));
    spawn_button(
        root,
        HostBtn,
        "HOST GAME",
        Color::srgba(0.2, 0.4, 0.25, 0.95),
        220.0,
        56.0,
    );
    spawn_button(
        root,
        JoinBtn,
        "JOIN GAME",
        Color::srgba(0.2, 0.3, 0.5, 0.95),
        220.0,
        56.0,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_hosting(
    root: &mut ChildBuilder,
    code: &str,
    config: &OnlineMatchConfig,
    roster: &[RosterEntry],
    worst_ping_ms: Option<u32>,
    selected_level: usize,
    score_target: Option<u32>,
) {
    // "Code: AAAA" on one line keeps the header compact.
    root.spawn(NodeBundle {
        style: Style {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            ..default()
        },
        ..default()
    })
    .with_children(|row| {
        row.spawn(TextBundle::from_section(
            "Code:",
            TextStyle {
                font_size: 14.0,
                color: Color::srgba(1.0, 1.0, 1.0, 0.75),
                ..default()
            },
        ));
        row.spawn((
            TextBundle::from_section(
                code.to_string(),
                TextStyle {
                    font_size: 36.0,
                    color: Color::srgb(1.0, 1.0, 0.4),
                    ..default()
                },
            ),
            CodeText,
        ));
    });

    paint_team_columns(root, roster, /*you_are_host=*/ true);

    if let Some(p) = worst_ping_ms {
        root.spawn(TextBundle::from_section(
            format!("Worst ping: {p}ms"),
            TextStyle {
                font_size: 14.0,
                color: Color::srgba(1.0, 1.0, 1.0, 0.7),
                ..default()
            },
        ));
    }

    // Level + Points share a row to save vertical space — landscape phone
    // in a lobby was getting cramped with each pickerline.
    root.spawn(NodeBundle {
        style: Style {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        },
        ..default()
    })
    .with_children(|row| {
        paint_level_cycle(row, selected_level);
        paint_score_picker(row, score_target);
    });
    paint_bot_picker(root, config);

    spawn_button(
        root,
        StartBtn,
        "START",
        Color::srgba(0.25, 0.5, 0.3, 0.95),
        220.0,
        56.0,
    );
}

fn paint_joining(root: &mut ChildBuilder, buf: &str) {
    root.spawn(TextBundle::from_section(
        "Enter the host's 4-letter code:",
        TextStyle {
            font_size: 16.0,
            color: Color::srgba(1.0, 1.0, 1.0, 0.75),
            ..default()
        },
    ));
    // Code display — show typed letters with underscores for missing slots.
    let mut display = String::new();
    for i in 0..4 {
        if i > 0 {
            display.push(' ');
        }
        display.push(buf.chars().nth(i).unwrap_or('_'));
    }
    root.spawn(TextBundle::from_section(
        display,
        TextStyle {
            font_size: 56.0,
            color: Color::srgb(1.0, 1.0, 0.4),
            ..default()
        },
    ));

    paint_letter_grid(root);

    if buf.len() == 4 {
        spawn_button(
            root,
            ConnectBtn,
            "CONNECT",
            Color::srgba(0.25, 0.5, 0.3, 0.95),
            220.0,
            56.0,
        );
    }
}

fn paint_joined(
    root: &mut ChildBuilder,
    _role: LobbyRole,
    roster: &[RosterEntry],
    worst_ping_ms: Option<u32>,
) {
    root.spawn(TextBundle::from_section(
        "Joined lobby — waiting for host to start…",
        TextStyle {
            font_size: 18.0,
            color: Color::srgba(1.0, 1.0, 1.0, 0.85),
            ..default()
        },
    ));
    paint_team_columns(root, roster, /*you_are_host=*/ false);
    if let Some(p) = worst_ping_ms {
        root.spawn(TextBundle::from_section(
            format!("Worst ping: {p}ms"),
            TextStyle {
                font_size: 14.0,
                color: Color::srgba(1.0, 1.0, 1.0, 0.7),
                ..default()
            },
        ));
    }
}

/// Two-column team display. The host is implicit (always Red, slot 0,
/// shown as "YOU (host)" or "Player 1 (host)" depending on whether the
/// current viewer is the host).
fn paint_team_columns(root: &mut ChildBuilder, roster: &[RosterEntry], you_are_host: bool) {
    root.spawn(NodeBundle {
        style: Style {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(20.0),
            ..default()
        },
        ..default()
    })
    .with_children(|row| {
        for (label, color, is_blue_team) in [
            ("RED TEAM", Color::srgb(1.0, 0.5, 0.5), false),
            ("BLUE TEAM", Color::srgb(0.5, 0.7, 1.0), true),
        ] {
            row.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    min_width: Val::Px(180.0),
                    ..default()
                },
                background_color: Color::srgba(0.08, 0.08, 0.18, 0.8).into(),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            })
            .with_children(|col| {
                col.spawn(TextBundle::from_section(
                    label,
                    TextStyle { font_size: 18.0, color, ..default() },
                ));
                // Host is always on Red.
                if !is_blue_team {
                    let host_label = if you_are_host { "YOU (host)" } else { "Player 1 (host)" };
                    col.spawn(TextBundle::from_section(
                        host_label.to_string(),
                        TextStyle { font_size: 16.0, color: Color::WHITE, ..default() },
                    ));
                }
                for entry in roster.iter().filter(|r| r.is_blue == is_blue_team) {
                    col.spawn(TextBundle::from_section(
                        format!("Player {}", entry.display_num),
                        TextStyle { font_size: 16.0, color: Color::WHITE, ..default() },
                    ));
                }
            });
        }
    });
}

fn paint_bot_picker(root: &mut ChildBuilder, config: &OnlineMatchConfig) {
    for (label, team, count, current_diff) in [
        ("Red bots", BotTeam::Red, config.red_bots, config.red_difficulty),
        ("Blue bots", BotTeam::Blue, config.blue_bots, config.blue_difficulty),
    ] {
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
            row.spawn(NodeBundle {
                style: Style { width: Val::Px(90.0), ..default() },
                ..default()
            })
            .with_children(|c| {
                c.spawn(TextBundle::from_section(
                    label,
                    TextStyle { font_size: 14.0, color: Color::WHITE, ..default() },
                ));
            });
            spawn_button(
                row,
                BotAdjBtn { team, delta: -1 },
                "-",
                Color::srgba(0.25, 0.25, 0.45, 0.95),
                36.0,
                32.0,
            );
            row.spawn((
                TextBundle::from_section(
                    format!("{}", count),
                    TextStyle { font_size: 20.0, color: Color::WHITE, ..default() },
                ),
                BotCountText(team),
            ));
            spawn_button(
                row,
                BotAdjBtn { team, delta: 1 },
                "+",
                Color::srgba(0.25, 0.25, 0.45, 0.95),
                36.0,
                32.0,
            );
            // Inline difficulty picker — three small E/M/H buttons.
            // Active difficulty is highlighted brighter.
            for diff in [
                crate::bot::BotDifficulty::Easy,
                crate::bot::BotDifficulty::Medium,
                crate::bot::BotDifficulty::Hard,
            ] {
                let active = diff == current_diff;
                let bg = if active {
                    Color::srgba(0.45, 0.45, 0.7, 0.95)
                } else {
                    Color::srgba(0.2, 0.2, 0.4, 0.9)
                };
                let label = match diff {
                    crate::bot::BotDifficulty::Easy => "E",
                    crate::bot::BotDifficulty::Medium => "M",
                    crate::bot::BotDifficulty::Hard => "H",
                };
                spawn_button(
                    row,
                    TeamDifficultyBtn { team, diff },
                    label,
                    bg,
                    32.0,
                    32.0,
                );
            }
        });
    }
}

fn paint_level_cycle(root: &mut ChildBuilder, selected_level: usize) {
    let level_name = crate::game::LEVELS
        .get(selected_level)
        .map(|l| l.name)
        .unwrap_or("?");
    // ASCII-only label — Bevy's default font doesn't include glyphs like
    // ▶ / ∞, so they render as empty squares on most devices. Bundling
    // a Unicode-capable font would be the proper fix but isn't worth a
    // ~1MB asset just for two arrows.
    spawn_button(
        root,
        LevelCycleBtn,
        &format!("Level: {}  >", level_name),
        Color::srgba(0.25, 0.25, 0.45, 0.95),
        260.0,
        34.0,
    );
}

fn paint_score_picker(root: &mut ChildBuilder, score_target: Option<u32>) {
    let label = match score_target {
        Some(n) => format!("Points: {n}"),
        None => "Points: UNL".to_string(),
    };
    root.spawn(NodeBundle {
        style: Style {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            padding: UiRect::all(Val::Px(4.0)),
            ..default()
        },
        background_color: Color::srgba(0.12, 0.12, 0.22, 0.8).into(),
        border_radius: BorderRadius::all(Val::Px(6.0)),
        ..default()
    })
    .with_children(|row| {
        spawn_button(
            row,
            ScoreAdjBtn(-1),
            "-",
            Color::srgba(0.25, 0.25, 0.45, 0.95),
            32.0,
            32.0,
        );
        row.spawn(NodeBundle {
            style: Style {
                width: Val::Px(120.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            ..default()
        })
        .with_children(|c| {
            c.spawn(TextBundle::from_section(
                label,
                TextStyle { font_size: 16.0, color: Color::WHITE, ..default() },
            ));
        });
        spawn_button(
            row,
            ScoreAdjBtn(1),
            "+",
            Color::srgba(0.25, 0.25, 0.45, 0.95),
            32.0,
            32.0,
        );
        spawn_button(
            row,
            ScoreUnlimitedBtn,
            "UNL",
            Color::srgba(0.3, 0.2, 0.5, 0.95),
            46.0,
            32.0,
        );
    });
}

fn paint_letter_grid(root: &mut ChildBuilder) {
    // 26 letters in 4 rows (7+7+7+5). Plus a backspace.
    let rows: [&str; 4] = ["ABCDEFG", "HIJKLMN", "OPQRSTU", "VWXYZ"];
    for row_letters in rows.iter() {
        root.spawn(NodeBundle {
            style: Style {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                ..default()
            },
            ..default()
        })
        .with_children(|row| {
            for ch in row_letters.chars() {
                spawn_button(
                    row,
                    LetterBtn(ch),
                    &ch.to_string(),
                    Color::srgba(0.18, 0.18, 0.32, 0.95),
                    44.0,
                    44.0,
                );
            }
        });
    }
    spawn_button(
        root,
        BackspaceBtn,
        "DEL",
        Color::srgba(0.35, 0.2, 0.3, 0.95),
        80.0,
        36.0,
    );
}

fn spawn_button<C: Component>(
    parent: &mut ChildBuilder,
    marker: C,
    label: &str,
    bg: Color,
    width: f32,
    height: f32,
) {
    parent
        .spawn((
            ButtonBundle {
                style: Style {
                    width: Val::Px(width),
                    height: Val::Px(height),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                background_color: bg.into(),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                focus_policy: FocusPolicy::Block,
                ..default()
            },
            marker,
        ))
        .with_children(|b| {
            b.spawn(TextBundle::from_section(
                label.to_string(),
                TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
            ));
        });
}

// ---------- Click handlers (split to stay under Bevy's 16-param limit) ----------

#[allow(clippy::too_many_arguments)]
fn handle_choose_buttons(
    mut commands: Commands,
    mut view: ResMut<LobbyView>,
    mut buf: ResMut<JoinCodeBuffer>,
    mut host_code: ResMut<HostLobbyCode>,
    mut role: ResMut<LobbyRole>,
    mut signaling_failed: ResMut<crate::net::SignalingFailed>,
    host_q: Query<&Interaction, (Changed<Interaction>, With<HostBtn>)>,
    join_q: Query<&Interaction, (Changed<Interaction>, With<JoinBtn>)>,
) {
    for i in &host_q {
        if *i == Interaction::Pressed {
            // Reset stale failure flag from a prior session — without this,
            // bail_on_signaling_failure can fire immediately and bounce
            // the user back to the menu before the new socket is even tried.
            signaling_failed.0 = false;
            let code = crate::net::LobbyCode::random();
            host_code.0 = code.0.clone();
            *role = LobbyRole::Host;
            commands.insert_resource(open_socket(&code.0));
            *view = LobbyView::Hosting;
        }
    }
    for i in &join_q {
        if *i == Interaction::Pressed {
            buf.0.clear();
            *role = LobbyRole::Client;
            *view = LobbyView::Joining;
        }
    }
}

fn handle_text_input(
    mut buf: ResMut<JoinCodeBuffer>,
    letter_q: Query<(&Interaction, &LetterBtn), Changed<Interaction>>,
    backspace_q: Query<&Interaction, (Changed<Interaction>, With<BackspaceBtn>)>,
) {
    for (i, l) in &letter_q {
        if *i == Interaction::Pressed && buf.0.len() < 4 {
            buf.0.push(l.0);
        }
    }
    for i in &backspace_q {
        if *i == Interaction::Pressed {
            buf.0.pop();
        }
    }
}

fn handle_bot_picker(
    mut config: ResMut<OnlineMatchConfig>,
    bot_q: Query<(&Interaction, &BotAdjBtn), Changed<Interaction>>,
) {
    for (i, adj) in &bot_q {
        if *i == Interaction::Pressed {
            let val = match adj.team {
                BotTeam::Red => &mut config.red_bots,
                BotTeam::Blue => &mut config.blue_bots,
            };
            *val = (*val as i32 + adj.delta).clamp(0, 4) as u32;
        }
    }
}

fn handle_difficulty_picker(
    mut config: ResMut<OnlineMatchConfig>,
    diff_q: Query<(&Interaction, &TeamDifficultyBtn), Changed<Interaction>>,
) {
    for (i, btn) in &diff_q {
        if *i != Interaction::Pressed {
            continue;
        }
        match btn.team {
            BotTeam::Red if config.red_difficulty != btn.diff => {
                config.red_difficulty = btn.diff;
            }
            BotTeam::Blue if config.blue_difficulty != btn.diff => {
                config.blue_difficulty = btn.diff;
            }
            _ => {}
        }
    }
}

fn handle_level_picker(
    mut selected_level: ResMut<crate::game::SelectedLevel>,
    q: Query<&Interaction, (Changed<Interaction>, With<LevelCycleBtn>)>,
) {
    for i in &q {
        if *i == Interaction::Pressed {
            selected_level.0 = (selected_level.0 + 1) % crate::game::LEVELS.len();
        }
    }
}

fn handle_score_picker(
    mut score_target: ResMut<crate::game::ScoreTarget>,
    adj_q: Query<(&Interaction, &ScoreAdjBtn), Changed<Interaction>>,
    unl_q: Query<&Interaction, (Changed<Interaction>, With<ScoreUnlimitedBtn>)>,
) {
    for (i, adj) in &adj_q {
        if *i == Interaction::Pressed {
            let current = score_target.0.unwrap_or(5) as i32;
            score_target.0 = Some((current + adj.0).clamp(1, 15) as u32);
        }
    }
    for i in &unl_q {
        if *i == Interaction::Pressed {
            score_target.0 = match score_target.0 {
                Some(_) => None,
                None => Some(5),
            };
        }
    }
}

fn handle_connect_button(
    mut commands: Commands,
    mut view: ResMut<LobbyView>,
    buf: Res<JoinCodeBuffer>,
    mut signaling_failed: ResMut<crate::net::SignalingFailed>,
    connect_q: Query<&Interaction, (Changed<Interaction>, With<ConnectBtn>)>,
) {
    for i in &connect_q {
        if *i == Interaction::Pressed && buf.0.len() == 4 {
            signaling_failed.0 = false;
            commands.insert_resource(open_socket(&buf.0));
            *view = LobbyView::Joined;
        }
    }
}

fn handle_back_button(
    mut commands: Commands,
    mut role: ResMut<LobbyRole>,
    mut next: ResMut<NextState<AppState>>,
    back_q: Query<&Interaction, (Changed<Interaction>, With<BackBtn>)>,
) {
    for i in &back_q {
        if *i == Interaction::Pressed {
            commands.remove_resource::<NetSocket>();
            *role = LobbyRole::None;
            next.set(AppState::Menu);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_start_button(
    mut config: ResMut<OnlineMatchConfig>,
    mut bot_counts: ResMut<crate::game::BotCounts>,
    mut net_mode: ResMut<NetworkMode>,
    mut local_net_id: ResMut<LocalNetId>,
    mut peer_slots: ResMut<PeerSlots>,
    mut diff: ResMut<crate::game::SelectedDifficulty>,
    mut next: ResMut<NextState<AppState>>,
    role: Res<LobbyRole>,
    peers: Res<ConnectedPeers>,
    socket: Option<ResMut<NetSocket>>,
    selected_level: Res<crate::game::SelectedLevel>,
    score_target: Res<crate::game::ScoreTarget>,
    start_q: Query<&Interaction, (Changed<Interaction>, With<StartBtn>)>,
) {
    let pressed = start_q.iter().any(|i| *i == Interaction::Pressed);
    if !pressed || *role != LobbyRole::Host {
        return;
    }

    // Stamp host's main-menu choices into the config before computing
    // the team setup. compute_match_setup preserves these fields.
    config.level = selected_level.0;
    config.score_target = score_target.0;

    let (new_config, assignments, slots) = compute_match_setup(&peers, &config);
    *config = new_config;
    bot_counts.red_humans = config.red_humans;
    bot_counts.blue_humans = config.blue_humans;
    bot_counts.red_allies = config.red_bots;
    bot_counts.blue_enemies = config.blue_bots;
    bot_counts.red_difficulty = config.red_difficulty;
    bot_counts.blue_difficulty = config.blue_difficulty;
    // SelectedDifficulty isn't used by spawn_ships anymore (per-team
    // diff comes from BotCounts), but keep it in sync with red so any
    // legacy reader gets a sensible value.
    diff.0 = config.red_difficulty;
    peer_slots.0 = slots;
    *local_net_id = LocalNetId(1);
    info!(
        "host: STARTING match — assignments={:?}",
        assignments.iter().map(|a| (a.peer_id.as_str(), a.net_id)).collect::<Vec<_>>()
    );

    if let Some(mut s) = socket {
        broadcast(
            &mut s,
            &peers,
            &NetMessage::Lobby(LobbyEvent::Start {
                config: *config,
                assignments,
            }),
        );
    }
    *net_mode = NetworkMode::OnlineHost;
    next.set(AppState::Playing);
}

/// Build the roster list the host shows + broadcasts. Does NOT include
/// the host (host is shown separately in the UI as "YOU").
fn build_roster(peers: &ConnectedPeers) -> Vec<RosterEntry> {
    sorted_peer_ids(peers)
        .iter()
        .enumerate()
        .map(|(idx, pid)| RosterEntry {
            peer_id: pid.to_string(),
            is_blue: is_blue_for_index(idx),
            display_num: (idx as u32) + 2, // host is "Player 1"
        })
        .collect()
}

/// Client-side: drain the lobby inbox for a Start message, learn our
/// NetId from the assignments, transition to Playing.
#[allow(clippy::too_many_arguments)]
fn receive_start_from_host(
    socket: Option<ResMut<NetSocket>>,
    role: Res<LobbyRole>,
    mut inbox: ResMut<LobbyInbox>,
    mut config: ResMut<OnlineMatchConfig>,
    mut bot_counts: ResMut<crate::game::BotCounts>,
    mut diff: ResMut<crate::game::SelectedDifficulty>,
    mut selected_level: ResMut<crate::game::SelectedLevel>,
    mut score_target: ResMut<crate::game::ScoreTarget>,
    mut net_mode: ResMut<NetworkMode>,
    mut local_net_id: ResMut<LocalNetId>,
    mut host_peer: ResMut<HostPeerId>,
    mut next: ResMut<NextState<AppState>>,
) {
    if *role != LobbyRole::Client {
        inbox.starts.clear();
        return;
    }
    let Some(mut s) = socket else {
        inbox.starts.clear();
        return;
    };
    let my_id = s.0.id().map(|p| p.to_string()).unwrap_or_default();
    let starts = std::mem::take(&mut inbox.starts);
    for (peer, c, assignments) in starts {
        *config = c;
        bot_counts.red_humans = c.red_humans;
        bot_counts.blue_humans = c.blue_humans;
        bot_counts.red_allies = c.red_bots;
        bot_counts.blue_enemies = c.blue_bots;
        bot_counts.red_difficulty = c.red_difficulty;
        bot_counts.blue_difficulty = c.blue_difficulty;
        diff.0 = c.red_difficulty;
        // Mirror host's level + score target so we play the same arena
        // and to the same goal. Without this clients use their own
        // local SelectedLevel / ScoreTarget which can differ silently.
        selected_level.0 = c.level;
        score_target.0 = c.score_target;
        if let Some(a) = assignments.iter().find(|a| a.peer_id == my_id) {
            *local_net_id = LocalNetId(a.net_id);
            info!("client: my peer_id={my_id} → LocalNetId={}", a.net_id);
        } else {
            warn!(
                "host sent Start with no assignment for our peer_id {}; \
                 assignments contained {:?}; defaulting to slot 1",
                my_id,
                assignments.iter().map(|a| a.peer_id.as_str()).collect::<Vec<_>>()
            );
            *local_net_id = LocalNetId(1);
        }
        host_peer.0 = Some(peer);
        *net_mode = NetworkMode::OnlineClient;
        next.set(AppState::Playing);
        return;
    }
}

/// Client-side: pick up roster snapshots so the lobby UI can render the
/// host's view of teams in real time.
fn receive_roster(
    role: Res<LobbyRole>,
    mut inbox: ResMut<LobbyInbox>,
    mut roster: ResMut<CurrentRoster>,
) {
    if *role != LobbyRole::Client {
        inbox.rosters.clear();
        return;
    }
    if let Some(latest) = inbox.rosters.drain(..).last() {
        if roster.0 != latest {
            roster.0 = latest;
        }
    }
}

/// Host-side: rebuild the local roster whenever the connected peer set
/// changes, mirror it to `CurrentRoster` so the host's own UI shows it,
/// and broadcast to all clients so they show the same thing.
fn host_broadcast_roster(
    role: Res<LobbyRole>,
    peers: Res<ConnectedPeers>,
    socket: Option<ResMut<NetSocket>>,
    mut roster: ResMut<CurrentRoster>,
    mut last_broadcast: Local<Vec<RosterEntry>>,
) {
    if *role != LobbyRole::Host {
        return;
    }
    let current = build_roster(&peers);
    if *last_broadcast == current {
        return;
    }
    *last_broadcast = current.clone();
    roster.0 = current.clone();
    if let Some(mut s) = socket {
        broadcast(
            &mut s,
            &peers,
            &NetMessage::Lobby(LobbyEvent::Roster { peers: current }),
        );
    }
}
