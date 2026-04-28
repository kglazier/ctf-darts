use bevy::input::touch::Touches;
use bevy::prelude::*;

use crate::movement::{MaxSpeed, Velocity};
use crate::player::{PlayerControlled, Stamina, Thrusting};
use crate::GameSet;

/// Per-ship input. The local player has one auto-populated by joystick &
/// keyboard. In online play, remote players will carry one fed by their
/// peer's `ClientInput` packets, so the same gameplay systems can read
/// inputs uniformly regardless of source.
#[derive(Component, Default, Clone, Copy)]
pub struct PlayerInput {
    pub move_dir: Vec2,
    pub sprint: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PointerId {
    Mouse,
    Touch(u64),
}

#[derive(Default)]
pub struct StickState {
    pub active_pointer: Option<PointerId>,
    pub offset: Vec2, // y-up, length <= 1
}

#[derive(Resource, Default)]
pub struct JoystickState {
    pub left: StickState,
    pub sprint_active: bool,
    pub sprint_pointer: Option<PointerId>,
}

#[derive(Resource, Default)]
pub struct JoystickLayout {
    pub left_center: Vec2,
    pub sprint_center: Vec2,
    pub base_radius: f32,
    pub sprint_radius: f32,
}

#[derive(Component)]
pub struct JoystickThumb;

#[derive(Component)]
pub struct SprintButton;

#[derive(Component)]
pub struct SprintButtonText;

pub const BASE_SIZE: f32 = 160.0;
pub const THUMB_SIZE: f32 = 64.0;
pub const SPRINT_SIZE: f32 = 140.0;
pub const INSET: f32 = 40.0;

pub struct InputPlugin;
impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<JoystickState>()
            .init_resource::<JoystickLayout>()
            .add_systems(
                Update,
                (
                    reset_input,
                    update_layout,
                    update_pointers,
                    update_joystick_visuals,
                    read_keyboard,
                    sticks_to_input,
                )
                    .chain()
                    .in_set(GameSet::Input),
            )
            .add_systems(Update, apply_input_to_player.in_set(GameSet::Ai));
    }
}

fn reset_input(mut q: Query<&mut PlayerInput, With<PlayerControlled>>) {
    for mut input in &mut q {
        *input = PlayerInput::default();
    }
}

fn update_layout(mut layout: ResMut<JoystickLayout>, windows: Query<&Window>) {
    let Ok(w) = windows.get_single() else { return };
    let ww = w.width();
    let wh = w.height();
    let half_base = BASE_SIZE * 0.5;
    let half_sprint = SPRINT_SIZE * 0.5;
    layout.left_center = Vec2::new(INSET + half_base, wh - INSET - half_base);
    layout.sprint_center = Vec2::new(ww - INSET - half_sprint, wh - INSET - half_sprint);
    layout.base_radius = half_base;
    layout.sprint_radius = half_sprint;
}

pub fn spawn_joystick_ui(mut commands: Commands) {
    let base_color = Color::srgba(0.2, 0.25, 0.35, 0.35);
    let thumb_color = Color::srgba(0.75, 0.85, 0.95, 0.7);
    let sprint_idle = Color::srgba(0.9, 0.5, 0.3, 0.55);

    // Left joystick base (move)
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(INSET),
                    left: Val::Px(INSET),
                    width: Val::Px(BASE_SIZE),
                    height: Val::Px(BASE_SIZE),
                    ..default()
                },
                background_color: base_color.into(),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            crate::game::PlayingEntity,
        ))
        .with_children(|p| {
            p.spawn((
                NodeBundle {
                    style: Style {
                        position_type: PositionType::Absolute,
                        left: Val::Px((BASE_SIZE - THUMB_SIZE) * 0.5),
                        top: Val::Px((BASE_SIZE - THUMB_SIZE) * 0.5),
                        width: Val::Px(THUMB_SIZE),
                        height: Val::Px(THUMB_SIZE),
                        ..default()
                    },
                    background_color: thumb_color.into(),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                JoystickThumb,
            ));
        });

    // Sprint/boost button — bottom-right, big and finger-friendly, labelled.
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(INSET),
                    right: Val::Px(INSET),
                    width: Val::Px(SPRINT_SIZE),
                    height: Val::Px(SPRINT_SIZE),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                background_color: sprint_idle.into(),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            SprintButton,
            crate::game::PlayingEntity,
        ))
        .with_children(|p| {
            p.spawn((
                TextBundle::from_section(
                    "BOOST",
                    TextStyle { font_size: 26.0, color: Color::WHITE, ..default() },
                ),
                SprintButtonText,
            ));
        });
}

fn update_pointers(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    windows: Query<&Window>,
    layout: Res<JoystickLayout>,
    mut state: ResMut<JoystickState>,
) {
    let Ok(window) = windows.get_single() else { return };

    if !mouse_buttons.pressed(MouseButton::Left) {
        release_pointer(&mut state, PointerId::Mouse);
    }
    for t in touches.iter_just_released() {
        release_pointer(&mut state, PointerId::Touch(t.id()));
    }
    for t in touches.iter_just_canceled() {
        release_pointer(&mut state, PointerId::Touch(t.id()));
    }

    if mouse_buttons.just_pressed(MouseButton::Left) {
        if let Some(pos) = window.cursor_position() {
            assign_pointer(&mut state, &layout, PointerId::Mouse, pos);
        }
    }
    for t in touches.iter_just_pressed() {
        assign_pointer(&mut state, &layout, PointerId::Touch(t.id()), t.position());
    }

    let mouse_pos = if mouse_buttons.pressed(MouseButton::Left) {
        window.cursor_position()
    } else {
        None
    };

    if let Some(p) = state.left.active_pointer {
        if let Some(cur) = pointer_pos(p, mouse_pos, &touches) {
            state.left.offset = stick_offset(cur, layout.left_center, layout.base_radius);
        }
    } else {
        state.left.offset = Vec2::ZERO;
    }

    if state.sprint_pointer.is_none() {
        state.sprint_active = false;
    }
}

fn pointer_pos(ptr: PointerId, mouse_pos: Option<Vec2>, touches: &Touches) -> Option<Vec2> {
    match ptr {
        PointerId::Mouse => mouse_pos,
        PointerId::Touch(id) => touches.get_pressed(id).map(|t| t.position()),
    }
}

fn stick_offset(current: Vec2, center: Vec2, base_radius: f32) -> Vec2 {
    let mut d = current - center;
    d.y = -d.y;
    (d / base_radius).clamp_length_max(1.0)
}

fn release_pointer(state: &mut JoystickState, pointer: PointerId) {
    if state.left.active_pointer == Some(pointer) {
        state.left.active_pointer = None;
        state.left.offset = Vec2::ZERO;
    }
    if state.sprint_pointer == Some(pointer) {
        state.sprint_pointer = None;
        state.sprint_active = false;
    }
}

fn assign_pointer(
    state: &mut JoystickState,
    layout: &JoystickLayout,
    pointer: PointerId,
    pos: Vec2,
) {
    let left_dist = pos.distance(layout.left_center);
    let sprint_dist = pos.distance(layout.sprint_center);

    if sprint_dist <= layout.sprint_radius {
        state.sprint_active = true;
        state.sprint_pointer = Some(pointer);
    } else if left_dist <= layout.base_radius * 1.3 && state.left.active_pointer.is_none() {
        state.left.active_pointer = Some(pointer);
    }
}

fn update_joystick_visuals(
    state: Res<JoystickState>,
    mut thumbs: Query<&mut Style, With<JoystickThumb>>,
    mut sprint: Query<&mut BackgroundColor, With<SprintButton>>,
) {
    let amp = (BASE_SIZE - THUMB_SIZE) * 0.5;
    for mut style in &mut thumbs {
        style.left = Val::Px(amp + state.left.offset.x * amp);
        style.top = Val::Px(amp - state.left.offset.y * amp);
    }
    if let Ok(mut bg) = sprint.get_single_mut() {
        bg.0 = if state.sprint_active {
            Color::srgba(1.0, 0.65, 0.35, 0.95)
        } else {
            Color::srgba(0.9, 0.5, 0.3, 0.55)
        };
    }
}

fn read_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut q: Query<&mut PlayerInput, With<PlayerControlled>>,
) {
    let Ok(mut input) = q.get_single_mut() else { return };
    let mut mv = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) { mv.y += 1.0; }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) { mv.y -= 1.0; }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) { mv.x -= 1.0; }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) { mv.x += 1.0; }
    if mv.length_squared() > 0.0 {
        input.move_dir = mv.normalize_or_zero();
    }
    if keys.pressed(KeyCode::Space) || keys.pressed(KeyCode::ShiftLeft) {
        input.sprint = true;
    }
}

pub fn sticks_to_input(
    state: Res<JoystickState>,
    mut q: Query<&mut PlayerInput, With<PlayerControlled>>,
) {
    let Ok(mut input) = q.get_single_mut() else { return };
    if state.left.offset.length_squared() > 0.04 {
        input.move_dir = state.left.offset;
    }
    if state.sprint_active {
        input.sprint = true;
    }
}

pub fn apply_input_to_player(
    mode: Res<crate::projectile::GameMode>,
    time: Res<Time>,
    mut q: Query<
        (
            &PlayerInput,
            &mut Velocity,
            &MaxSpeed,
            &mut Thrusting,
            &mut Stamina,
            Option<&crate::flag::CarryingFlag>,
        ),
        // All human-driven ships: the local PlayerControlled AND online
        // RemoteHumans (whose PlayerInput is fed by network packets via
        // host_apply_remote_input on the host side). Bots are excluded
        // since they have BotDifficulty and are driven by drive_bots.
        Without<crate::bot::BotDifficulty>,
    >,
) {
    for (input, mut vel, max_speed, mut thrusting, mut stamina, carrying) in &mut q {

    // ACCEL must be > DRAG * (max_speed * SPRINT_MUL) or drag-equilibrium
    // pulls the player below the boost cap, leaving bots (which only have
    // a hard cap, no drag) faster than the boosted player. With DRAG=3 and
    // a 320*1.8=576 boosted cap, equilibrium needs ACCEL > 1728. Old value
    // (1400) gave equilibrium ~466 and a permanently slower player.
    const ACCEL: f32 = 2000.0;
    const DRAG: f32 = 3.0;
    // Boost is weaker while carrying — carriers are the vulnerable party.
    const SPRINT_MUL: f32 = 1.8;
    const SPRINT_MUL_CARRY: f32 = 1.4;
    const SPRINT_DRAIN: f32 = 0.7;

    let dt = time.delta_seconds();
    // Boost only exists in Classic mode; in Shooter mode the button fires instead.
    let wants_sprint =
        *mode == crate::projectile::GameMode::Classic && input.sprint && stamina.current > 0.05;
    thrusting.0 = wants_sprint;
    let sprint_mul = if carrying.is_some() { SPRINT_MUL_CARRY } else { SPRINT_MUL };
    let current_max = if wants_sprint {
        stamina.current = (stamina.current - dt * SPRINT_DRAIN).max(0.0);
        max_speed.0 * sprint_mul
    } else {
        max_speed.0
    };

    if input.move_dir.length_squared() > 0.01 {
        vel.0 += input.move_dir * ACCEL * dt;
    }
    vel.0 *= (1.0 - DRAG * dt).max(0.0);
    if vel.0.length() > current_max {
        vel.0 = vel.0.normalize() * current_max;
    }
    }
}
