use bevy::prelude::*;
use bevy_matchbox::matchbox_socket::{PeerId, PeerState, SingleChannel, WebRtcSocket};
use bevy_matchbox::MatchboxSocket;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::flag::{Flag, FlagState, Score};
use crate::input::PlayerInput as InputComponent;
use crate::movement::Velocity;
use crate::player::{Facing, Ship, Stamina, Thrusting};
use crate::projectile::Projectile;
use crate::tag::Respawning;
use crate::team::Team;

/// How the current match is being played. Solo and OnlineHost both run the
/// authoritative simulation locally; OnlineClient applies snapshots from a
/// remote host.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum NetworkMode {
    #[default]
    Solo,
    OnlineHost,
    OnlineClient,
}

impl NetworkMode {
    /// True when this peer runs the authoritative sim (Solo or OnlineHost).
    /// Gameplay system sets are gated on this so OnlineClient renders from
    /// snapshots instead of stepping AI/Physics/Gameplay locally.
    pub fn is_local_authority(self) -> bool {
        matches!(self, NetworkMode::Solo | NetworkMode::OnlineHost)
    }
}

/// Where to find the matchbox signaling server. The room code is appended
/// to form a per-lobby URL so peers entering the same code land in the same
/// room. Override at runtime by setting the `SPACE_BOOSTERS_SIGNAL` env var.
///
/// IMPORTANT: this URL must match the matchbox protocol version of the
/// `bevy_matchbox` crate in Cargo.toml. helsing.studio publishes one
/// signaling server per matchbox version (`match-0-10` for matchbox 0.10,
/// `match-0-9` for 0.9, etc). Mismatched versions accept the connection
/// then close it as soon as a real message arrives, which surfaces as a
/// panic at `matchbox_socket/src/webrtc_socket/socket.rs:477` ("called
/// Result::unwrap() on an Err value: Closed"). When you upgrade
/// `bevy_matchbox`, update the URL too — or, better, deploy your own
/// signaling server (see signaling/README.md).
// Back to the hostname-based URL. Now that bindProcessToNetwork is
// returning true at SDK 35 (per ImmersiveActivity logs), DNS via libc
// should work and TLS routing on Fly's shared IPv4 should resolve via
// the SNI hostname. This is the "proper" URL we'd ship long-term.
pub const DEFAULT_SIGNAL_URL: &str = "wss://space-boosters-signaling-kg.fly.dev";

/// Lobby code = 4 uppercase letters, easy to read aloud and type. Random
/// generation uses the OS clock as seed so we don't add a `rand` crate just
/// for this.
#[derive(Resource, Clone, Debug)]
pub struct LobbyCode(pub String);

impl LobbyCode {
    pub fn random() -> Self {
        // Tiny LCG seeded from system time. Plenty for a 4-letter code; not
        // remotely cryptographic — collisions are user-visible ("code
        // already in use, try again") rather than security-critical.
        // web_time::SystemTime works on native AND wasm; std::time panics
        // on wasm32-unknown-unknown.
        use web_time::{SystemTime, UNIX_EPOCH};
        let mut seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xDEADBEEF);
        let mut s = String::with_capacity(4);
        for _ in 0..4 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let idx = ((seed >> 33) % 26) as u8;
            s.push((b'A' + idx) as char);
        }
        Self(s)
    }
}

/// Local peer's role within a lobby.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LobbyRole {
    #[default]
    None,
    Host,
    Client,
}

/// Live socket. None when not in a lobby. Wrapped so Bevy can store it as a
/// resource and the underlying socket can be dropped/recreated on
/// disconnect.
#[derive(Resource)]
pub struct NetSocket(pub MatchboxSocket<SingleChannel>);

/// Connected peer table — populated by `poll_socket` from PeerState events.
#[derive(Resource, Default)]
pub struct ConnectedPeers {
    pub peers: HashMap<PeerId, PeerInfo>,
}

#[derive(Clone, Debug)]
pub struct PeerInfo {
    /// True once the WebRTC data channel is open in both directions.
    pub connected: bool,
}

/// Lobby-time configuration the host picks before launching the match. The
/// host serializes this and sends `LobbyEvent::Start { config }` to all
/// peers; the client mirrors the same config locally so spawn_ships
/// produces matching ship sets and so everyone plays on the same level
/// to the same score.
#[derive(Resource, Clone, Copy, Debug, Serialize, Deserialize)]
pub struct OnlineMatchConfig {
    pub red_humans: u32,
    pub blue_humans: u32,
    pub red_bots: u32,
    pub blue_bots: u32,
    pub red_difficulty: crate::bot::BotDifficulty,
    pub blue_difficulty: crate::bot::BotDifficulty,
    /// Index into `crate::game::LEVELS`. Host picks; broadcast on START
    /// so clients spawn the same arena.
    pub level: usize,
    /// `None` = unlimited (freeplay). Same shape as solo's ScoreTarget.
    pub score_target: Option<u32>,
}

impl Default for OnlineMatchConfig {
    fn default() -> Self {
        Self {
            red_humans: 1,
            blue_humans: 1,
            red_bots: 1,
            blue_bots: 1,
            red_difficulty: crate::bot::BotDifficulty::Medium,
            blue_difficulty: crate::bot::BotDifficulty::Medium,
            level: 0,
            score_target: Some(5),
        }
    }
}

/// Stable cross-peer identity for any replicable entity (ships, flags).
/// spawn_ships assigns these deterministically in spawn order, which both
/// host and client run with the same MatchConfig — so NetId(7) on the host
/// and on the client always refer to "the same ship". Snapshots are keyed
/// by NetId; clients look up local entities by NetId and apply the wire
/// state to them.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NetId(pub u32);

/// All wire messages share this envelope so peers can ignore unknown variants
/// from a future patch without crashing — bincode will refuse silently and
/// we just drop the packet.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum NetMessage {
    /// Client → host every input frame.
    ClientInput { seq: u32, input: WireInput },
    /// Host → all clients, ~30Hz with current world snapshot.
    HostSnapshot(SnapshotPayload),
    /// Lobby control plane.
    Lobby(LobbyEvent),
    /// Round-trip ping for the lobby ping display (S4).
    Ping { sent_ms: u64 },
    Pong { sent_ms: u64 },
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SnapshotPayload {
    pub seq: u32,
    /// Host's monotonic time at snapshot — clients use this for
    /// interpolation timing (S3 task 9).
    pub host_time: f32,
    pub ships: Vec<ShipSnap>,
    pub flags: Vec<FlagSnap>,
    pub projectiles: Vec<ProjectileSnap>,
    pub score_red: u32,
    pub score_blue: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShipSnap {
    pub net_id: u32,
    pub team: u8, // 0 = Red, 1 = Blue
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub facing: f32,
    pub thrusting: bool,
    pub stamina: f32,
    /// 0 if alive; otherwise seconds remaining on respawn timer.
    pub respawning_secs: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FlagSnap {
    pub team: u8, // 0 = Red, 1 = Blue
    pub state: FlagSnapState,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum FlagSnapState {
    Home,
    Carried { net_id: u32 },
    Dropped { x: f32, y: f32 },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProjectileSnap {
    pub team: u8,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct WireInput {
    pub move_x: f32,
    pub move_y: f32,
    pub sprint: bool,
}

impl From<InputComponent> for WireInput {
    fn from(i: InputComponent) -> Self {
        Self { move_x: i.move_dir.x, move_y: i.move_dir.y, sprint: i.sprint }
    }
}

impl From<WireInput> for InputComponent {
    fn from(w: WireInput) -> Self {
        Self { move_dir: Vec2::new(w.move_x, w.move_y), sprint: w.sprint }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum LobbyEvent {
    /// Host announces the match is starting. Includes the agreed config so
    /// every peer's spawn_ships produces an identical roster, plus the
    /// peer→NetId assignment so each client learns which ship is "me".
    Start {
        config: OnlineMatchConfig,
        /// One entry per remote peer the host has accepted. Each client
        /// finds itself in this list (PeerId equality) to set LocalNetId.
        /// Host is implicit: NetId 1 (red slot 0).
        assignments: Vec<PeerAssignment>,
    },
    /// Pre-start lobby snapshot — host broadcasts whenever the connected
    /// peer set changes so every client can render the current Red/Blue
    /// team layout in real time.
    Roster {
        peers: Vec<RosterEntry>,
    },
    /// Host triggers a restart from GameOver. Carries fresh assignments
    /// (peers that dropped during GameOver are now bots) plus a countdown
    /// so all peers show the same "STARTING IN: N" overlay.
    Restart {
        config: OnlineMatchConfig,
        assignments: Vec<PeerAssignment>,
        countdown_secs: f32,
    },
    /// Host added a bot mid-match. NetId is host-chosen so clients spawn
    /// with the same identifier and snapshot replication aligns from the
    /// next tick onward.
    AddBot {
        is_blue: bool,
        net_id: u32,
        difficulty: crate::bot::BotDifficulty,
    },
    /// Host removed a bot mid-match. Each client finds the ship with this
    /// NetId locally and despawns it.
    RemoveBot {
        net_id: u32,
    },
    /// Host changed a bot's difficulty mid-match (chip tap on the bot HUD).
    /// Bot AI runs host-authoritatively, so this exists purely so clients
    /// can keep their HUD chip labels in sync.
    SetBotDifficulty {
        net_id: u32,
        difficulty: crate::bot::BotDifficulty,
    },
    /// Host changed an ally bot's mode (Auto/Offense/Defense). Same rationale
    /// as SetBotDifficulty — clients only need it for HUD parity.
    SetBotMode {
        net_id: u32,
        mode: crate::bot::AllyMode,
    },
    /// A peer is leaving the match cleanly (vs. silent disconnect).
    Leave,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PeerAssignment {
    /// Peer's id as a string (PeerId is UUID-shaped). String over the wire
    /// avoids depending on PeerId's serde impl staying stable.
    pub peer_id: String,
    pub net_id: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub peer_id: String,
    /// false = Red, true = Blue. Same alternation rule as the Start
    /// handler so the visual layout matches the eventual NetId mapping.
    pub is_blue: bool,
    /// Display number for the lobby UI ("Player 2"). Stable per-peer for
    /// the duration of the lobby; not the same as NetId.
    pub display_num: u32,
}

/// Inbox for lobby control-plane messages. `poll_socket` routes
/// `LobbyEvent` values here; lobby systems drain by type and act on them.
/// Solves the race where both `poll_socket` and a dedicated lobby
/// receiver would call `socket.receive()` and steal each other's packets.
#[derive(Resource, Default)]
pub struct LobbyInbox {
    pub starts: Vec<(PeerId, OnlineMatchConfig, Vec<PeerAssignment>)>,
    pub rosters: Vec<Vec<RosterEntry>>,
    pub restarts: Vec<(OnlineMatchConfig, Vec<PeerAssignment>, f32)>,
    pub bot_adds: Vec<(bool /* is_blue */, u32 /* net_id */, crate::bot::BotDifficulty)>,
    pub bot_removes: Vec<u32>,
    pub bot_difficulty_sets: Vec<(u32 /* net_id */, crate::bot::BotDifficulty)>,
    pub bot_mode_sets: Vec<(u32 /* net_id */, crate::bot::AllyMode)>,
}

/// Host-side queue for `ClientInput` packets routed from `poll_socket`.
/// Without this, both `poll_socket` and `host_apply_remote_input` race to
/// drain the matchbox receive queue — whoever loses the race silently
/// drops the input, leading to sluggish/missed remote-player movement.
#[derive(Resource, Default)]
pub struct ClientInputInbox {
    pub queue: Vec<(PeerId, WireInput)>,
}

/// Active restart countdown. Set by host on PLAY AGAIN, mirrored on
/// clients via `LobbyEvent::Restart`. Ticks down in GameOver state; when
/// it expires both peers transition to Restarting.
#[derive(Resource, Default)]
pub struct RestartCountdown(pub Option<Timer>);

/// Sort peers deterministically (by string PeerId) so all peers agree on
/// the ordering used for team alternation. HashMap iteration order is
/// non-deterministic; without this, peers could compute different team
/// layouts from the same data.
pub fn sorted_peer_ids(peers: &ConnectedPeers) -> Vec<PeerId> {
    let mut v: Vec<_> = peers.peers.keys().copied().collect();
    v.sort_by_key(|p| p.to_string());
    v
}

/// `idx` is the peer's position in the sorted list (host not included).
/// Even indices go to Blue so a 2-player lobby is host-Red vs peer-Blue
/// instead of two players on Red and an empty Blue.
pub fn is_blue_for_index(idx: usize) -> bool {
    idx % 2 == 0
}

/// Compute the full match setup (humans+bots per team, NetId assignments
/// per peer, peer→NetId map) from currently-connected peers and the
/// host's chosen config. Used by both lobby Start and GameOver restart
/// so the same alternation rule applies to both flows.
///
/// "Bots replace humans" rule: total ships per team = base config's
/// (humans + bots). When peers are missing, the human slots become bots,
/// keeping total team size stable.
pub fn compute_match_setup(
    peers: &ConnectedPeers,
    base_config: &OnlineMatchConfig,
) -> (OnlineMatchConfig, Vec<PeerAssignment>, HashMap<PeerId, u32>) {
    let peer_ids = sorted_peer_ids(peers);
    let mut red_humans = 1u32; // host
    let mut blue_humans = 0u32;
    let mut team_for_peer: Vec<bool> = Vec::with_capacity(peer_ids.len());
    for idx in 0..peer_ids.len() {
        let is_blue = is_blue_for_index(idx);
        team_for_peer.push(is_blue);
        if is_blue {
            blue_humans += 1;
        } else {
            red_humans += 1;
        }
    }

    // Preserve the original team total. base_config is what the host
    // picked when they pressed START the first time; if anyone has
    // dropped since, lost humans become extra bots so the team is still
    // full.
    let original_red_total = base_config.red_humans + base_config.red_bots;
    let original_blue_total = base_config.blue_humans + base_config.blue_bots;
    let mut new_config = *base_config;
    new_config.red_humans = red_humans;
    new_config.blue_humans = blue_humans;
    new_config.red_bots = original_red_total.saturating_sub(red_humans);
    new_config.blue_bots = original_blue_total.saturating_sub(blue_humans);

    // NetId layout per spawn_ships:
    //   Red humans (1..=red_humans)  ← host is NetId 1
    //   Red bots
    //   Blue humans
    //   Blue bots
    let red_human_first = 1u32;
    let blue_human_first = red_humans + new_config.red_bots + 1;

    let mut next_red_slot = 1u32; // host occupies slot 0 → NetId 1
    let mut next_blue_slot = 0u32;
    let mut assignments: Vec<PeerAssignment> = Vec::with_capacity(peer_ids.len());
    let mut slots_map: HashMap<PeerId, u32> = HashMap::new();
    for (idx, pid) in peer_ids.iter().enumerate() {
        let net_id = if team_for_peer[idx] {
            let n = blue_human_first + next_blue_slot;
            next_blue_slot += 1;
            n
        } else {
            let n = red_human_first + next_red_slot;
            next_red_slot += 1;
            n
        };
        assignments.push(PeerAssignment {
            peer_id: pid.to_string(),
            net_id,
        });
        slots_map.insert(*pid, net_id);
    }

    (new_config, assignments, slots_map)
}

/// "Which NetId is the local player's ship." Solo + OnlineHost = 1.
/// OnlineClient sets this from the matching `PeerAssignment` in
/// `LobbyEvent::Start`. spawn_ships reads this to decide which entity gets
/// the `PlayerControlled` marker.
#[derive(Resource, Clone, Copy, Debug)]
pub struct LocalNetId(pub u32);

impl Default for LocalNetId {
    fn default() -> Self {
        Self(1)
    }
}

/// Host-side: peer→NetId map so received `ClientInput` packets can be
/// routed to the correct ship.
#[derive(Resource, Default)]
pub struct PeerSlots(pub HashMap<PeerId, u32>);

/// Round-trip latency to each connected peer, in milliseconds. Updated
/// every ~2s by the ping system; consumed by the lobby UI to show whether
/// any peer is on a bad connection.
#[derive(Resource, Default)]
pub struct PeerPings(pub HashMap<PeerId, u32>);

#[derive(Resource)]
pub struct PingTimer(pub Timer);

impl Default for PingTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(2.0, TimerMode::Repeating))
    }
}

/// Client-side record of which peer is the host. Set when the client
/// receives `LobbyEvent::Start` (the peer that sent it is by definition
/// the host). Used to detect host-disconnect and bail to menu.
#[derive(Resource, Default)]
pub struct HostPeerId(pub Option<PeerId>);

/// Emitted when a peer disconnects mid-match. Host's bot-takeover system
/// listens; client's host-disconnect system listens.
#[derive(Event, Debug, Clone, Copy)]
pub struct PeerLeftEvent {
    pub peer: PeerId,
}

/// Set true when the matchbox socket's internal task has died (signaling
/// connection lost, panic in matchbox internals, etc). A separate system
/// reads this to tear down the socket and bail to the menu — without
/// this the next call into matchbox would panic and kill the app.
#[derive(Resource, Default)]
pub struct SignalingFailed(pub bool);

/// Build the per-room signaling URL. `code` is the human-typeable lobby
/// code; we append it to the signal base so peers in the same room cluster
/// at matchbox.
fn room_url(code: &str) -> String {
    let base = std::env::var("SPACE_BOOSTERS_SIGNAL")
        .unwrap_or_else(|_| DEFAULT_SIGNAL_URL.to_string());
    format!("{}/space-boosters-{}", base.trim_end_matches('/'), code)
}

/// Open a socket against the room URL for `code`. Caller stashes the
/// returned socket as `NetSocket` resource. Both host and client use the
/// same call — matchbox doesn't distinguish; the LobbyRole resource
/// records who runs the sim.
pub fn open_socket(code: &str) -> NetSocket {
    // bevy_matchbox accepts (socket, message_loop) and spawns the loop on
    // its own task pool. We hand it the pair via From.
    let pair = WebRtcSocket::new_reliable(room_url(code));
    NetSocket(MatchboxSocket::from(pair))
}

/// 60Hz snapshot send rate. ~20 KB/s per peer at 6 ships, well under any
/// modern broadband connection. Higher rate gives smoother remote-ship
/// visuals and tighter misprediction correction (16ms vs 33ms snapback).
/// If a future player on a constrained network has trouble, drop to 30
/// here and they'll get a more forgiving experience at the cost of some
/// smoothness.
pub const SNAPSHOT_HZ: f32 = 60.0;

#[derive(Resource)]
pub struct SnapshotTimer(pub Timer);

impl Default for SnapshotTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(1.0 / SNAPSHOT_HZ, TimerMode::Repeating))
    }
}

#[derive(Resource, Default)]
pub struct SnapshotSeq(pub u32);

/// Client-side ring buffer of recent snapshots (capacity 4, ~133ms at 30Hz).
/// Two adjacent snapshots are needed for linear interpolation between them;
/// extra slack covers small reorder/delay before we drop old frames.
#[derive(Resource, Default)]
pub struct SnapshotBuffer {
    pub recent: std::collections::VecDeque<SnapshotPayload>,
}

/// Maps host's snapshot timestamps to the client's local clock so render
/// time advances smoothly between snapshot arrivals (rather than jumping
/// only when a snapshot arrives, which causes 30Hz stutter at 60fps
/// rendering). Calibrated on first snapshot; reset when host changes.
#[derive(Resource, Default)]
pub struct ClientClock {
    /// `host_time` corresponds to local `elapsed_seconds() - offset`.
    /// `None` until the first snapshot arrives.
    pub offset: Option<f32>,
}

/// How far behind the freshest snapshot the client renders, in seconds.
/// One snapshot interval (33ms) plus a small cushion (17ms) keeps us with
/// at least one snapshot already past the render time, so interpolation
/// always has both endpoints. Lower = more responsive but more jitter.
pub const RENDER_DELAY_SECS: f32 = 0.05;

pub struct NetPlugin;
impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetworkMode>()
            .init_resource::<LobbyRole>()
            .init_resource::<ConnectedPeers>()
            .init_resource::<OnlineMatchConfig>()
            .init_resource::<SnapshotTimer>()
            .init_resource::<SnapshotSeq>()
            .init_resource::<SnapshotBuffer>()
            .init_resource::<ClientClock>()
            .init_resource::<LocalNetId>()
            .init_resource::<PeerSlots>()
            .init_resource::<HostPeerId>()
            .init_resource::<PeerPings>()
            .init_resource::<PingTimer>()
            .init_resource::<LobbyInbox>()
            .init_resource::<ClientInputInbox>()
            .init_resource::<RestartCountdown>()
            .init_resource::<SignalingFailed>()
            .add_event::<PeerLeftEvent>()
            .add_systems(Update, poll_socket.run_if(socket_present))
            .add_systems(Update, host_send_snapshot.run_if(should_send_snapshot))
            .add_systems(Update, client_apply_snapshot.run_if(is_online_client))
            .add_systems(Update, client_detect_host_timeout.run_if(is_online_client))
            // client_send_input must run AFTER the Input chain has populated
            // PlayerInput for this frame; otherwise it ships either last
            // frame's value or (worse) the zeroed value from reset_input,
            // depending on Bevy's scheduling. Without this ordering the host
            // sees roughly half the packets as zero-input → remote ship
            // crawls instead of moving at full speed.
            .add_systems(
                Update,
                client_send_input
                    .run_if(is_online_client)
                    .after(crate::input::sticks_to_input),
            )
            // Same reasoning as client_send_input: must run AFTER the Input
            // chain populates PlayerInput, otherwise prediction sees the
            // value reset_input just zeroed and the local ship doesn't
            // move on the player's own screen even though their input
            // gets shipped to the host correctly.
            .add_systems(
                Update,
                client_predict_local
                    .run_if(is_online_client)
                    .after(crate::input::sticks_to_input),
            )
            // Pickup + tag prediction run AFTER local prediction so they
            // see the ship at its updated position. The host stays
            // authoritative — snapshot apply reverts mispredictions.
            // Standard pattern (TagPro, Valve, Gambetta); misprediction
            // is visible as a brief snapback or flicker, which is the
            // accepted tradeoff for responsive gameplay.
            .add_systems(
                Update,
                (client_predict_pickup, client_predict_tag)
                    .after(client_predict_local)
                    .run_if(is_online_client),
            )
            .add_systems(Update, host_apply_remote_input.run_if(is_online_host))
            .add_systems(
                Update,
                client_apply_bot_changes
                    .run_if(is_online_client)
                    .run_if(in_state(crate::game::AppState::Playing)),
            )
            .add_systems(
                Update,
                host_delay_local_input
                    .run_if(is_online_host)
                    .before(crate::input::apply_input_to_player),
            )
            .add_systems(Update, client_handle_host_disconnect.run_if(is_online_client))
            .add_systems(Update, host_replace_disconnected_with_bot.run_if(is_online_host))
            .add_systems(Update, ping_send_timer.run_if(socket_present))
            .add_systems(Update, bail_on_signaling_failure);
    }
}

fn should_send_snapshot(
    mode: Res<NetworkMode>,
    socket: Option<Res<NetSocket>>,
) -> bool {
    *mode == NetworkMode::OnlineHost && socket.is_some()
}

fn is_online_client(mode: Res<NetworkMode>) -> bool {
    *mode == NetworkMode::OnlineClient
}

fn is_online_host(mode: Res<NetworkMode>) -> bool {
    *mode == NetworkMode::OnlineHost
}

/// `run_if` predicate: true on Solo and OnlineHost. Used in `lib.rs` to
/// gate gameplay system sets so OnlineClient skips local simulation and
/// applies snapshots instead.
pub fn is_local_authority(mode: Res<NetworkMode>) -> bool {
    mode.is_local_authority()
}

fn socket_present(socket: Option<Res<NetSocket>>) -> bool {
    socket.is_some()
}

/// Drain matchbox's peer-state events into our `ConnectedPeers` table and
/// route incoming packets: `HostSnapshot` goes into the SnapshotBuffer for
/// the client apply system; LobbyEvent::Start is consumed in `lobby.rs` via
/// its own receive system (which calls `socket.receive()` directly there
/// since lobby flow precedes the snapshot stream).
#[allow(clippy::too_many_arguments)]
fn poll_socket(
    mut socket: ResMut<NetSocket>,
    mut peers: ResMut<ConnectedPeers>,
    mut snapshots: ResMut<SnapshotBuffer>,
    mut pings: ResMut<PeerPings>,
    mut lobby_inbox: ResMut<LobbyInbox>,
    mut client_input_inbox: ResMut<ClientInputInbox>,
    mut left_events: EventWriter<PeerLeftEvent>,
    mut signaling_failed: ResMut<SignalingFailed>,
    mode: Res<NetworkMode>,
    time: Res<Time>,
) {
    // matchbox panics from inside its message-loop task if the underlying
    // signaling connection fails (DNS / TLS / version mismatch). Wrap each
    // call so we can detect that and gracefully tear down rather than
    // crashing the whole app. Without this, a single network blip kills
    // the game; with it, the player gets bounced to the main menu.
    let peer_changes = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        socket.0.update_peers()
    })) {
        Ok(v) => v,
        Err(_) => {
            warn!("matchbox panicked in update_peers; signaling has failed");
            signaling_failed.0 = true;
            return;
        }
    };
    for (peer, state) in peer_changes {
        match state {
            PeerState::Connected => {
                peers.peers.insert(peer, PeerInfo { connected: true });
                info!("peer connected: {peer}");
            }
            PeerState::Disconnected => {
                peers.peers.remove(&peer);
                pings.0.remove(&peer);
                info!("peer disconnected: {peer}");
                left_events.send(PeerLeftEvent { peer });
            }
        }
    }
    let received: Vec<(PeerId, Box<[u8]>)> =
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| socket.0.receive())) {
            Ok(v) => v,
            Err(_) => {
                warn!("matchbox panicked in receive; signaling has failed");
                signaling_failed.0 = true;
                return;
            }
        };
    let now_ms = (time.elapsed_seconds() * 1000.0) as u64;
    for (peer, bytes) in received {
        let Ok(msg) = bincode::deserialize::<NetMessage>(&bytes) else {
            warn!("decode error from {peer:?}");
            continue;
        };
        match msg {
            NetMessage::HostSnapshot(payload) if *mode == NetworkMode::OnlineClient => {
                let stale = snapshots
                    .recent
                    .back()
                    .map(|s| payload.seq <= s.seq)
                    .unwrap_or(false);
                if !stale {
                    snapshots.recent.push_back(payload);
                    while snapshots.recent.len() > 4 {
                        snapshots.recent.pop_front();
                    }
                }
            }
            NetMessage::ClientInput { input, .. } if *mode == NetworkMode::OnlineHost => {
                // Route into the input inbox so host_apply_remote_input
                // can drain it. Without this, the message is silently
                // dropped here — host_apply_remote_input would never see
                // it because we already drained the receive queue above.
                client_input_inbox.queue.push((peer, input));
            }
            NetMessage::Ping { sent_ms } => {
                // Echo back so the sender can measure RTT.
                if let Ok(bytes) = bincode::serialize(&NetMessage::Pong { sent_ms }) {
                    socket.0.send(bytes.into_boxed_slice(), peer);
                }
            }
            NetMessage::Pong { sent_ms } => {
                let rtt = now_ms.saturating_sub(sent_ms) as u32;
                pings.0.insert(peer, rtt);
            }
            NetMessage::Lobby(LobbyEvent::Start { config, assignments }) => {
                lobby_inbox.starts.push((peer, config, assignments));
            }
            NetMessage::Lobby(LobbyEvent::Roster { peers }) => {
                lobby_inbox.rosters.push(peers);
            }
            NetMessage::Lobby(LobbyEvent::Restart {
                config,
                assignments,
                countdown_secs,
            }) => {
                lobby_inbox
                    .restarts
                    .push((config, assignments, countdown_secs));
            }
            NetMessage::Lobby(LobbyEvent::AddBot { is_blue, net_id, difficulty }) => {
                lobby_inbox.bot_adds.push((is_blue, net_id, difficulty));
            }
            NetMessage::Lobby(LobbyEvent::RemoveBot { net_id }) => {
                lobby_inbox.bot_removes.push(net_id);
            }
            NetMessage::Lobby(LobbyEvent::SetBotDifficulty { net_id, difficulty }) => {
                lobby_inbox.bot_difficulty_sets.push((net_id, difficulty));
            }
            NetMessage::Lobby(LobbyEvent::SetBotMode { net_id, mode }) => {
                lobby_inbox.bot_mode_sets.push((net_id, mode));
            }
            NetMessage::Lobby(LobbyEvent::Leave) => {
                // Treat as an early disconnect signal — actual peer-left
                // event still fires when WebRTC times out.
                debug!("peer {peer:?} sent Leave");
            }
            other => debug!("from {peer:?}: {other:?}"),
        }
    }
}

fn ping_send_timer(
    time: Res<Time>,
    mut timer: ResMut<PingTimer>,
    mut socket: ResMut<NetSocket>,
    peers: Res<ConnectedPeers>,
) {
    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }
    let sent_ms = (time.elapsed_seconds() * 1000.0) as u64;
    let msg = NetMessage::Ping { sent_ms };
    let Ok(bytes) = bincode::serialize(&msg) else {
        return;
    };
    for (peer, info) in &peers.peers {
        if info.connected {
            socket.0.send(bytes.clone().into_boxed_slice(), *peer);
        }
    }
}

/// Encode + send a message to a single peer. Returns `false` if the peer is
/// unknown or the channel is closed; gameplay code can use that to surface
/// "lost connection to host" without poking matchbox internals.
pub fn send_to(socket: &mut NetSocket, peer: PeerId, msg: &NetMessage) -> bool {
    match bincode::serialize(msg) {
        Ok(bytes) => {
            socket.0.send(bytes.into_boxed_slice(), peer);
            true
        }
        Err(e) => {
            warn!("encode error: {e}");
            false
        }
    }
}

/// Broadcast to every connected peer. Skips peers whose channel hasn't
/// finished negotiating yet.
pub fn broadcast(socket: &mut NetSocket, peers: &ConnectedPeers, msg: &NetMessage) {
    let Ok(bytes) = bincode::serialize(msg) else {
        return;
    };
    for (peer, info) in &peers.peers {
        if info.connected {
            socket.0.send(bytes.clone().into_boxed_slice(), *peer);
        }
    }
}

fn team_to_u8(t: Team) -> u8 {
    match t {
        Team::Red => 0,
        Team::Blue => 1,
    }
}

#[allow(dead_code)] // Used by S3 task 10 input replication on the host.
fn u8_to_team(b: u8) -> Team {
    if b == 0 {
        Team::Red
    } else {
        Team::Blue
    }
}

/// Artificial input lag for the host's own ship, in seconds. Originally
/// 25ms to equalize against client RTT lag in 50/50 contests. Now that
/// clients run client-side prediction (their local view is instant too),
/// this delay only hurts the host without helping anyone — both sides
/// perceive their own actions immediately. Set to 0; keep the constant
/// (and the buffering system) so it's easy to dial back up if a future
/// playtest shows host advantage in trades.
pub const HOST_INPUT_DELAY_SECS: f32 = 0.0;

/// Per-ship ring of recent input samples (timestamp + value). Only the
/// host's local-player ship actually uses it; we attach it to every Player
/// at spawn so we don't have to branch in spawn_ships on NetworkMode.
#[derive(Component, Default)]
pub struct HostInputDelayBuf {
    samples: std::collections::VecDeque<(f32, InputComponent)>,
}

/// Per-snapshot smoothing factor for the local ship's position — every
/// snapshot we lerp the local position this fraction of the way toward
/// the server's view. At 60Hz snapshots that pulls drift down by ~15%
/// per frame (~60% in 100ms), keeping client and server views aligned
/// without the visible "snap" you get from a hard threshold model.
/// This is what real games (Source, Valorant, TagPro) do — continuous
/// invisible correction rather than threshold-and-teleport. Bump up
/// (toward 0.3) for tighter alignment / faster correction; down for
/// smoother feel.
pub const RECONCILE_SMOOTH_FACTOR: f32 = 0.15;

/// Hard-snap fallback for huge drifts — respawn teleports, dramatic
/// network catch-ups. Smooth correction would slide the ship across
/// the map for these cases, which looks worse than just snapping.
pub const RECONCILE_SNAP_DIST: f32 = 200.0;

/// When the freshest snapshot is older than render time, extrapolate
/// remote ship positions using their last reported velocity. Caps at
/// this many seconds — beyond that the extrapolation is more harmful
/// than helpful (the target may have changed direction).
pub const EXTRAPOLATION_CAP_SECS: f32 = 0.15;

/// If we go this long without receiving any snapshot from the host,
/// assume the host has dropped (matchbox doesn't always fire a clean
/// disconnect event when the host quits abruptly) and bail to the menu.
pub const HOST_TIMEOUT_SECS: f32 = 5.0;

/// Client-side: pick the two snapshots straddling render time and lerp
/// every replicated entity between them. The local ship is skipped here
/// — `client_predict_local` owns it; we only reconcile if the host's view
/// has drifted too far from our prediction.
#[allow(clippy::too_many_arguments)]
fn client_apply_snapshot(
    time: Res<Time>,
    buffer: Res<SnapshotBuffer>,
    mut clock: ResMut<ClientClock>,
    local_id: Res<LocalNetId>,
    mut score: ResMut<Score>,
    mut ships: Query<(
        Entity,
        &NetId,
        &mut Transform,
        &mut Velocity,
        &mut Facing,
        &mut Thrusting,
        &mut Stamina,
        &mut Visibility,
    )>,
    mut flags: Query<&mut Flag>,
) {
    let Some(newest) = buffer.recent.back() else {
        return;
    };
    let local_now = time.elapsed_seconds();
    // Calibrate offset on first snapshot — host_time + offset ≈ local_now
    // for the freshest snapshot. After that, render_t advances with the
    // local clock so interpolation slides smoothly between snapshot
    // arrivals instead of stepping at the 30Hz snapshot rate.
    if clock.offset.is_none() {
        clock.offset = Some(local_now - newest.host_time);
    }
    // If the freshest snapshot is wildly newer than what our offset
    // predicts (e.g. host paused then resumed), re-calibrate forward so
    // we don't sit interpolating into ancient data.
    if let Some(off) = clock.offset {
        let predicted_local = newest.host_time + off;
        if local_now - predicted_local > 0.5 {
            clock.offset = Some(local_now - newest.host_time);
        }
    }
    let offset = clock.offset.unwrap();
    let render_t = local_now - offset - RENDER_DELAY_SECS;

    // Pick prev/next bracketing render_t. Walk from oldest forward.
    let mut prev: Option<&SnapshotPayload> = None;
    let mut next: Option<&SnapshotPayload> = None;
    for snap in &buffer.recent {
        if snap.host_time <= render_t {
            prev = Some(snap);
        } else if next.is_none() {
            next = Some(snap);
            break;
        }
    }

    let (a, b, t_lerp) = match (prev, next) {
        (Some(p), Some(n)) => {
            let span = (n.host_time - p.host_time).max(1e-4);
            let t = ((render_t - p.host_time) / span).clamp(0.0, 1.0);
            (p, n, t)
        }
        (Some(p), None) => (p, p, 0.0),
        (None, Some(n)) => (n, n, 0.0),
        (None, None) => return,
    };

    // If render_t is past the newest snapshot, extrapolate forward from
    // velocity (capped to EXTRAPOLATION_CAP_SECS) so remote ships keep
    // moving smoothly during a brief snapshot starvation rather than
    // freezing in place. Beyond the cap we hold position — better
    // honest stutter than wildly inaccurate extrapolation.
    let extrap_secs = (render_t - newest.host_time).clamp(0.0, EXTRAPOLATION_CAP_SECS);

    let by_id_a: HashMap<u32, &ShipSnap> = a.ships.iter().map(|s| (s.net_id, s)).collect();
    let by_id_b: HashMap<u32, &ShipSnap> = b.ships.iter().map(|s| (s.net_id, s)).collect();

    // Build net_id → Entity once so the flag carrier lookup below is O(1)
    // and doesn't fight the borrow checker mid-iteration.
    let netid_to_entity: HashMap<u32, Entity> = ships
        .iter()
        .map(|(e, n, _, _, _, _, _, _)| (n.0, e))
        .collect();

    for (_e, net_id, mut tf, mut vel, mut facing, mut thrust, mut stamina, mut vis) in &mut ships {
        let Some(sb) = by_id_b.get(&net_id.0) else {
            continue;
        };
        // Mirror visibility from respawn state — host hides ships during
        // their respawn window and unhides on revive. We don't replicate
        // visibility directly, but `respawning_secs > 0` is equivalent.
        *vis = if sb.respawning_secs > 0.0 {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if net_id.0 == local_id.0 {
            // Local ship: smooth toward an EXTRAPOLATED server position
            // — i.e., where the server would currently see this ship if
            // it kept moving at the snapshot's velocity for the time
            // since the snapshot was taken. Without this extrapolation,
            // smoothing pulls the ship toward a position that's already
            // ~RTT/2 stale, which feels like "sliding on ice" when the
            // user turns: their local prediction snaps to the new
            // direction but the smoothing yanks back toward the old
            // direction the server saw before they turned.
            let server_pos = Vec2::new(sb.x, sb.y);
            let server_vel = Vec2::new(sb.vx, sb.vy);
            let snapshot_age = (local_now - (newest.host_time + offset)).clamp(0.0, 0.2);
            let extrapolated = server_pos + server_vel * snapshot_age;
            let local_pos = Vec2::new(tf.translation.x, tf.translation.y);
            let dist = local_pos.distance(extrapolated);
            let new_pos = if dist > RECONCILE_SNAP_DIST || sb.respawning_secs > 0.0 {
                // Hard snap — respawn or major desync.
                vel.0 = server_vel;
                extrapolated
            } else {
                local_pos.lerp(extrapolated, RECONCILE_SMOOTH_FACTOR)
            };
            tf.translation.x = new_pos.x;
            tf.translation.y = new_pos.y;
            // Stamina is host-authoritative resource accounting — sync
            // every snapshot so prediction has the right "can boost" state.
            stamina.current = sb.stamina;
            continue;
        }
        let sa = by_id_a.get(&net_id.0).copied().unwrap_or(*sb);
        let xa = Vec2::new(sa.x, sa.y);
        let xb = Vec2::new(sb.x, sb.y);
        let mut pos = if xa.distance(xb) > 200.0 {
            xb
        } else {
            xa.lerp(xb, t_lerp)
        };
        // If we're past the newest snapshot, push forward by velocity to
        // bridge the gap without freezing. The local ship is exempt
        // (predicted separately above) and respawning ships are exempt
        // (they teleport rather than slide).
        if extrap_secs > 0.0 && sb.respawning_secs == 0.0 {
            pos += Vec2::new(sb.vx, sb.vy) * extrap_secs;
        }
        tf.translation.x = pos.x;
        tf.translation.y = pos.y;
        vel.0 = Vec2::new(sb.vx, sb.vy);
        facing.0 = lerp_angle(sa.facing, sb.facing, t_lerp);
        tf.rotation = Quat::from_rotation_z(facing.0);
        thrust.0 = sb.thrusting;
        stamina.current = sb.stamina;
    }

    for mut flag in &mut flags {
        let team_b = team_to_u8(flag.team);
        let Some(snap) = b.flags.iter().find(|f| f.team == team_b) else {
            continue;
        };
        flag.state = match &snap.state {
            FlagSnapState::Home => FlagState::Home,
            FlagSnapState::Dropped { x, y } => FlagState::Dropped(Vec2::new(*x, *y)),
            FlagSnapState::Carried { net_id } => match netid_to_entity.get(net_id) {
                Some(&e) => FlagState::Carried(e),
                // Carrier hasn't spawned locally yet (race during state
                // transition). Hold previous state; next snapshot will
                // reflect once the carrier exists on this client.
                None => flag.state,
            },
        };
    }

    score.red = b.score_red;
    score.blue = b.score_blue;
}

/// Client → host: serialize the local player's current input and send to
/// the host every frame. Throttle to ~60Hz: matchbox handles backpressure
/// fine but we don't gain anything from sending faster than the local
/// frame rate. A monotonic seq lets the host drop reordered packets if
/// WebRTC ever delivers out of order.
fn client_send_input(
    mut socket: ResMut<NetSocket>,
    peers: Res<ConnectedPeers>,
    mut seq: Local<u32>,
    local: Query<&InputComponent, With<crate::player::PlayerControlled>>,
) {
    let Ok(input) = local.get_single() else {
        return;
    };
    *seq = seq.wrapping_add(1);
    let msg = NetMessage::ClientInput { seq: *seq, input: (*input).into() };
    // The host is the only peer that matters. Broadcast is fine — other
    // clients ignore ClientInput in their poll handler.
    broadcast(&mut socket, &peers, &msg);
}

/// Host: drain queued `ClientInput` packets (routed in by `poll_socket`)
/// and write each sender's latest input into the corresponding remote-
/// human ship's `PlayerInput` component. The existing AI/Movement
/// systems then consume these inputs uniformly with the host's local
/// input — no special-case branching for remote vs. local players.
fn host_apply_remote_input(
    mut inbox: ResMut<ClientInputInbox>,
    slots: Res<PeerSlots>,
    mut ships: Query<(&NetId, &mut InputComponent)>,
) {
    let queue = std::mem::take(&mut inbox.queue);
    for (peer, input) in queue {
        let Some(&net_id) = slots.0.get(&peer) else {
            // Unknown peer — lingering packets from a peer that
            // disconnected before lobby Start. Drop silently.
            continue;
        };
        for (n, mut player_input) in &mut ships {
            if n.0 == net_id {
                *player_input = input.into();
                break;
            }
        }
    }
}

/// Client-side prediction for the local ship. Reuses the same physics
/// constants as `apply_input_to_player` AND runs the same wall-collision
/// resolution as the host so local movement feels responsive (no
/// ghosting through walls before snapshot-snaps you back). The host
/// remains authoritative — `client_apply_snapshot` reconciles if our
/// prediction has drifted beyond `RECONCILE_SNAP_DIST`.
#[allow(clippy::too_many_arguments)]
fn client_predict_local(
    time: Res<Time>,
    mode: Res<crate::projectile::GameMode>,
    walls: Query<(&Transform, &crate::arena::Wall), Without<Velocity>>,
    mut q: Query<
        (
            &InputComponent,
            &mut Transform,
            &mut Velocity,
            &crate::movement::MaxSpeed,
            &mut Facing,
            &mut Thrusting,
            &mut Stamina,
        ),
        (
            With<crate::player::PlayerControlled>,
            Without<crate::arena::Wall>,
            Without<Respawning>,
        ),
    >,
) {
    let Ok((input, mut tf, mut vel, max_speed, mut facing, mut thrust, mut stamina)) =
        q.get_single_mut()
    else {
        return;
    };
    let dt = time.delta_seconds();

    // These constants mirror `apply_input_to_player` in input.rs. If you
    // tune one set, tune both — drift between them shows up as constant
    // reconciliation snaps.
    const ACCEL: f32 = 2000.0;
    const DRAG: f32 = 3.0;
    const SPRINT_MUL: f32 = 1.8;
    const SPRINT_DRAIN: f32 = 0.7;

    let wants_sprint = *mode == crate::projectile::GameMode::Classic
        && input.sprint
        && stamina.current > 0.05;
    thrust.0 = wants_sprint;
    let current_max = if wants_sprint {
        stamina.current = (stamina.current - dt * SPRINT_DRAIN).max(0.0);
        max_speed.0 * SPRINT_MUL
    } else {
        max_speed.0
    };

    // Mirror apply_input_to_player: while boosting, normalize partial
    // joystick deflection so a touch player can reach the boost cap
    // without dragging the thumb to the joystick base edge.
    let thrust_dir = if wants_sprint && input.move_dir.length_squared() > 0.01 {
        input.move_dir.normalize()
    } else {
        input.move_dir
    };
    if thrust_dir.length_squared() > 0.01 {
        vel.0 += thrust_dir * ACCEL * dt;
        // Face the move direction so the ship sprite points where it's going.
        facing.0 = input.move_dir.y.atan2(input.move_dir.x);
        tf.rotation = Quat::from_rotation_z(facing.0);
    }
    vel.0 *= (1.0 - DRAG * dt).max(0.0);
    if vel.0.length() > current_max {
        vel.0 = vel.0.normalize() * current_max;
    }
    let displacement = vel.0 * dt;
    tf.translation.x += displacement.x;
    tf.translation.y += displacement.y;

    // Wall collision — same logic the host runs, applied locally so the
    // predicted ship stops at walls instead of ghosting through them and
    // snapping back when the snapshot arrives.
    let pos = Vec2::new(tf.translation.x, tf.translation.y);
    let (new_pos, new_vel) = crate::physics::resolve_against_walls(
        pos,
        vel.0,
        crate::player::SHIP_RADIUS,
        &walls,
    );
    tf.translation.x = new_pos.x;
    tf.translation.y = new_pos.y;
    vel.0 = new_vel;
}

/// Optimistic flag-pickup prediction on the client. When the local ship
/// touches a flag, immediately update local Flag.state — own dropped
/// flag returns home, enemy flag becomes carried. Score is NOT awarded
/// locally; that stays authoritative on the host. If the host disagrees
/// (e.g. the host saw us not touching the flag yet due to lag), the
/// next snapshot reverts our optimistic state.
#[allow(clippy::too_many_arguments)]
fn client_predict_pickup(
    mut commands: Commands,
    local_id: Res<LocalNetId>,
    mut flags: Query<(Entity, &mut Flag)>,
    ships: Query<
        (Entity, &Transform, &Ship, &NetId, Option<&crate::flag::CarryingFlag>),
        Without<Flag>,
    >,
) {
    use crate::flag::{CarryingFlag, FlagState, FLAG_RADIUS};
    use crate::player::SHIP_RADIUS;

    let Some((ship_entity, ship_tf, ship, _, carrying)) = ships
        .iter()
        .find(|(_, _, _, n, _)| n.0 == local_id.0)
    else {
        return;
    };
    let ship_pos = ship_tf.translation.truncate();
    let already_carrying = carrying.is_some();

    for (flag_entity, mut flag) in &mut flags {
        let flag_pos = match flag.state {
            FlagState::Home => flag.home,
            FlagState::Dropped(p) => p,
            FlagState::Carried(_) => continue,
        };
        if flag_pos.distance(ship_pos) > SHIP_RADIUS + FLAG_RADIUS {
            continue;
        }
        if ship.team == flag.team {
            // Own team — return on touch (only if dropped).
            if matches!(flag.state, FlagState::Dropped(_)) {
                flag.state = FlagState::Home;
            }
        } else if !already_carrying {
            // Enemy flag — pick up. Insert CarryingFlag locally so the
            // carry_flag system makes the flag follow the ship visually.
            flag.state = FlagState::Carried(ship_entity);
            commands.entity(ship_entity).insert(CarryingFlag(flag_entity));
            break;
        }
    }
}

/// Optimistic tag prediction on the client. When the local ship is
/// boosting and an enemy enters its forward cone within tag range, hide
/// the enemy locally so the user gets immediate feedback instead of
/// waiting ~snapshot_interval + RTT for the host to confirm. Snapshot
/// apply will un-hide if the host disagreed (causing a brief flicker —
/// the accepted artifact of optimistic prediction).
fn client_predict_tag(
    local_id: Res<LocalNetId>,
    mut ships: Query<
        (
            &Transform,
            &Ship,
            &NetId,
            &crate::player::Facing,
            &crate::player::Thrusting,
            Option<&crate::flag::CarryingFlag>,
            &mut Visibility,
        ),
    >,
) {
    use crate::tag::{TAG_CONE_HALF_ANGLE, TAG_RANGE};

    // Snapshot the local ship's tag-relevant state first so we can then
    // freely mutate visibility on enemies in the same query.
    let local_state = ships.iter().find_map(|(tf, s, n, f, t, c, _)| {
        if n.0 == local_id.0 {
            Some((tf.translation.truncate(), s.team, f.0, t.0, c.is_some()))
        } else {
            None
        }
    });
    let Some((my_pos, my_team, my_facing, my_thrust, my_carrying)) = local_state else {
        return;
    };
    // Carriers can't tag (matches host rule in tag.rs detect_tag).
    if my_carrying {
        return;
    }
    let forward = Vec2::new(my_facing.cos(), my_facing.sin());

    for (tf, s, n, _, _, target_carrying, mut vis) in &mut ships {
        if n.0 == local_id.0 || s.team == my_team {
            continue;
        }
        let pos = tf.translation.truncate();
        let diff = pos - my_pos;
        let dist = diff.length();
        if dist > TAG_RANGE || dist < 0.0001 {
            continue;
        }
        let target_carries = target_carrying.is_some();
        // Same eligibility rules the host uses:
        // - Carriers die on any contact (no boost, no cone needed).
        // - Non-carriers die only to a sprinting attacker in cone.
        if target_carries {
            // any contact OK
        } else if !my_thrust {
            continue;
        } else {
            let dir = diff / dist;
            let dot = forward.dot(dir).clamp(-1.0, 1.0);
            if dot.acos() > TAG_CONE_HALF_ANGLE {
                continue;
            }
        }
        // Predict the kill — visual only. The host's snapshot drives
        // respawning_secs and authoritative visibility; this just
        // bridges the latency window.
        *vis = Visibility::Hidden;
    }
}

/// Replace the local player's PlayerInput with the value sampled
/// `HOST_INPUT_DELAY_SECS` ago. Equalizes (most of) the host's input-lag
/// advantage in 50/50 contests.
fn host_delay_local_input(
    time: Res<Time>,
    mut q: Query<
        (&mut InputComponent, &mut HostInputDelayBuf),
        With<crate::player::PlayerControlled>,
    >,
) {
    let Ok((mut current, mut buf)) = q.get_single_mut() else {
        return;
    };
    let now = time.elapsed_seconds();
    let target_t = now - HOST_INPUT_DELAY_SECS;

    buf.samples.push_back((now, *current));

    // Pick the latest sample at or before target_t. The buffer is small
    // (at most a few frames at 60Hz) so a linear walk is cheap.
    let mut delayed = *current; // fallback while the buffer fills at match start
    for (t, v) in buf.samples.iter() {
        if *t <= target_t {
            delayed = *v;
        } else {
            break;
        }
    }
    *current = delayed;

    // Trim history older than the delay window (with a small cushion for
    // jitter) so the buffer doesn't grow unbounded.
    let oldest_keep = now - (HOST_INPUT_DELAY_SECS * 4.0).max(0.1);
    while buf
        .samples
        .front()
        .map(|(t, _)| *t < oldest_keep)
        .unwrap_or(false)
    {
        buf.samples.pop_front();
    }
}

/// Client-side fallback for host-disconnect: if no snapshot has arrived
/// for `HOST_TIMEOUT_SECS`, treat the host as gone and bail to menu.
/// matchbox doesn't always emit a clean PeerLeftEvent when the host
/// quits abruptly (force-close, network drop), so without this watchdog
/// the client can sit frozen in the now-dead match indefinitely.
fn client_detect_host_timeout(
    time: Res<Time>,
    buffer: Res<SnapshotBuffer>,
    mut last_seen: Local<Option<f32>>,
    mut signaling_failed: ResMut<SignalingFailed>,
) {
    // Track the seq of the latest snapshot we've seen — when it changes,
    // reset our timer; when it stops changing for too long, fail.
    let newest_seq = buffer.recent.back().map(|s| s.seq);
    let now = time.elapsed_seconds();
    match (newest_seq, *last_seen) {
        (Some(_seq), None) => *last_seen = Some(now),
        (Some(_seq), Some(_)) if buffer.is_changed() => *last_seen = Some(now),
        (Some(_), Some(t)) if now - t > HOST_TIMEOUT_SECS => {
            warn!(
                "no snapshot from host in {:.1}s; assuming disconnect",
                now - t
            );
            signaling_failed.0 = true;
            *last_seen = None;
        }
        _ => {}
    }
}

/// When the matchbox internals have died (DNS lookup failure, signaling
/// server unreachable, protocol mismatch), tear down the socket and
/// transition back to the menu so the app stays alive. Without this, the
/// next call into matchbox would re-panic and Bevy would crash.
///
/// Only bails when we're actually in an online match (NetworkMode is
/// OnlineHost or OnlineClient). If the user has just exited a match
/// and is back in the lobby (NetworkMode == Solo), the OLD socket's
/// async task can still panic from teardown after a few frames — that
/// would set the failed flag, and without this gate we'd kick the user
/// to the menu before they can host/join their next match.
fn bail_on_signaling_failure(
    mut commands: Commands,
    mut failed: ResMut<SignalingFailed>,
    mut net_mode: ResMut<NetworkMode>,
    mut role: ResMut<LobbyRole>,
    mut next: ResMut<NextState<crate::game::AppState>>,
    state: Res<State<crate::game::AppState>>,
) {
    if !failed.0 {
        return;
    }
    // Always clear the flag so it doesn't pile up — but only act on it
    // when we're in a live online match. In Solo / lobby pre-Start the
    // failure is from a dying old socket; ignoring it is safe.
    failed.0 = false;
    if matches!(*net_mode, NetworkMode::Solo) {
        warn!("signaling failed during Solo/lobby — ignoring (likely stale socket teardown)");
        return;
    }
    commands.remove_resource::<NetSocket>();
    *net_mode = NetworkMode::Solo;
    *role = LobbyRole::None;
    if !matches!(state.get(), crate::game::AppState::Menu) {
        next.set(crate::game::AppState::Menu);
    }
    warn!("signaling failed — returned to menu");
}

/// Client side: if the host drops, the match can't continue. Tear down
/// the socket and bounce back to the menu. (No reconnection — the host
/// holds all state, so resuming would mean rejoining a fresh game.)
fn client_handle_host_disconnect(
    mut commands: Commands,
    mut events: EventReader<PeerLeftEvent>,
    mut next: ResMut<NextState<crate::game::AppState>>,
    mut net_mode: ResMut<NetworkMode>,
    host_id: Res<HostPeerId>,
    state: Res<State<crate::game::AppState>>,
) {
    for ev in events.read() {
        if Some(ev.peer) == host_id.0 {
            info!("host disconnected — returning to menu");
            commands.remove_resource::<NetSocket>();
            *net_mode = NetworkMode::Solo;
            // Skip the transition if we're already on Menu (e.g. the
            // disconnect arrived after we'd already left voluntarily).
            if !matches!(state.get(), crate::game::AppState::Menu) {
                next.set(crate::game::AppState::Menu);
            }
        }
    }
}

/// Host side: when a peer drops mid-match, hand their ship to the AI so
/// the team isn't suddenly missing a player. We only need to attach the
/// AI driver components — the existing bot systems pick it up next frame.
#[allow(clippy::too_many_arguments)]
fn host_replace_disconnected_with_bot(
    mut commands: Commands,
    mut events: EventReader<PeerLeftEvent>,
    mut slots: ResMut<PeerSlots>,
    selected_diff: Res<crate::game::SelectedDifficulty>,
    ships: Query<(Entity, &NetId, &Ship)>,
) {
    for ev in events.read() {
        let Some(net_id) = slots.0.remove(&ev.peer) else {
            continue;
        };
        let Some((entity, _, ship)) = ships.iter().find(|(_, n, _)| n.0 == net_id) else {
            continue;
        };
        info!(
            "peer {} disconnected mid-match; bot is taking over net_id {}",
            ev.peer, net_id
        );

        // Insert AI driver components. Red side: ally bot with Auto mode so
        // it takes cues from the player. Blue side: plain enemy bot.
        let mut ec = commands.entity(entity);
        ec.insert((
            crate::bot::BotRole::Grabber,
            selected_diff.0,
        ));
        if matches!(ship.team, Team::Red) {
            ec.insert((crate::bot::AllyBot, crate::bot::AllyMode::default()));
        }
    }
}

/// Client-side mirror of the host's mid-match `+`/`-` bot adjustments.
/// Drains pending AddBot/RemoveBot events from the inbox and applies
/// them locally so subsequent snapshots have something to write into.
#[allow(clippy::too_many_arguments)]
fn client_apply_bot_changes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut inbox: ResMut<LobbyInbox>,
    existing: Query<(&Ship, &NetId, Option<&crate::bot::BotNumber>)>,
    bots: Query<(Entity, &Ship, &NetId), With<crate::bot::BotDifficulty>>,
    mut diffs: Query<(&NetId, &mut crate::bot::BotDifficulty)>,
    mut modes: Query<(&NetId, &mut crate::bot::AllyMode)>,
    labels: Query<(Entity, &crate::bot::LabelFor), With<crate::bot::BotNumberLabel>>,
) {
    let adds = std::mem::take(&mut inbox.bot_adds);
    for (is_blue, net_id, difficulty) in adds {
        let team = if is_blue { Team::Blue } else { Team::Red };
        // Skip if a ship already exists with this NetId (idempotent in
        // case the host's broadcast was retried).
        if existing.iter().any(|(_, n, _)| n.0 == net_id) {
            continue;
        }
        crate::player::add_bot(
            &mut commands,
            &mut meshes,
            &mut materials,
            team,
            difficulty,
            &existing,
            Some(net_id),
        );
    }
    let removes = std::mem::take(&mut inbox.bot_removes);
    for net_id in removes {
        if let Some((entity, _, _)) = bots.iter().find(|(_, _, n)| n.0 == net_id) {
            commands.entity(entity).despawn();
            for (label_e, label_for) in labels.iter() {
                if label_for.0 == entity {
                    commands.entity(label_e).despawn();
                }
            }
        }
    }
    let diff_sets = std::mem::take(&mut inbox.bot_difficulty_sets);
    for (net_id, difficulty) in diff_sets {
        if let Some((_, mut d)) = diffs.iter_mut().find(|(n, _)| n.0 == net_id) {
            *d = difficulty;
        }
    }
    let mode_sets = std::mem::take(&mut inbox.bot_mode_sets);
    for (net_id, mode) in mode_sets {
        if let Some((_, mut m)) = modes.iter_mut().find(|(n, _)| n.0 == net_id) {
            *m = mode;
        }
    }
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    // Take the short way around the circle.
    let mut diff = (b - a) % (2.0 * std::f32::consts::PI);
    if diff > std::f32::consts::PI {
        diff -= 2.0 * std::f32::consts::PI;
    } else if diff < -std::f32::consts::PI {
        diff += 2.0 * std::f32::consts::PI;
    }
    a + diff * t
}

#[allow(clippy::too_many_arguments)]
fn host_send_snapshot(
    time: Res<Time>,
    mut timer: ResMut<SnapshotTimer>,
    mut seq: ResMut<SnapshotSeq>,
    mut socket: ResMut<NetSocket>,
    peers: Res<ConnectedPeers>,
    score: Res<Score>,
    ships: Query<(
        Entity,
        &NetId,
        &Transform,
        &Velocity,
        &Ship,
        &Facing,
        &Thrusting,
        &Stamina,
        Option<&Respawning>,
    )>,
    flags: Query<&Flag>,
    projectiles: Query<(&Transform, &Velocity, &Projectile)>,
) {
    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }

    let mut payload = SnapshotPayload {
        seq: seq.0,
        host_time: time.elapsed_seconds(),
        score_red: score.red,
        score_blue: score.blue,
        ..default()
    };
    seq.0 = seq.0.wrapping_add(1);

    // Build entity → net_id once so flag carrier lookup is O(1).
    let entity_to_net: HashMap<Entity, u32> = ships
        .iter()
        .map(|(e, n, _, _, _, _, _, _, _)| (e, n.0))
        .collect();

    for (_e, net_id, tf, vel, ship, facing, thrust, stamina, resp) in &ships {
        payload.ships.push(ShipSnap {
            net_id: net_id.0,
            team: team_to_u8(ship.team),
            x: tf.translation.x,
            y: tf.translation.y,
            vx: vel.0.x,
            vy: vel.0.y,
            facing: facing.0,
            thrusting: thrust.0,
            stamina: stamina.current,
            respawning_secs: resp
                .map(|r| r.timer.duration().as_secs_f32() - r.timer.elapsed_secs())
                .unwrap_or(0.0),
        });
    }
    for flag in &flags {
        let state = match flag.state {
            FlagState::Home => FlagSnapState::Home,
            FlagState::Carried(carrier) => match entity_to_net.get(&carrier) {
                // Carrier exists — encode by net_id.
                Some(&net_id) => FlagSnapState::Carried { net_id },
                // Race: carrier was just despawned but flag hasn't updated
                // this frame. Treat as Home; next tick will reflect truth.
                None => FlagSnapState::Home,
            },
            FlagState::Dropped(p) => FlagSnapState::Dropped { x: p.x, y: p.y },
        };
        payload.flags.push(FlagSnap { team: team_to_u8(flag.team), state });
    }
    for (tf, vel, proj) in &projectiles {
        payload.projectiles.push(ProjectileSnap {
            team: team_to_u8(proj.team),
            x: tf.translation.x,
            y: tf.translation.y,
            vx: vel.0.x,
            vy: vel.0.y,
        });
    }

    broadcast(&mut socket, &peers, &NetMessage::HostSnapshot(payload));
}
