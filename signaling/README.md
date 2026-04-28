# Signaling server (matchbox)

Space Boosters' online mode uses [matchbox](https://github.com/johanhelsing/matchbox)
for WebRTC peer-to-peer connections. The signaling server only matches peers
into lobbies — once a connection is established, all gameplay traffic flows
P2P between players, so signaling bandwidth and CPU are negligible.

You do not need to run your own signaling server while testing locally if a
public matchbox server is reachable. For shipping, host your own so you
control uptime and aren't a guest on someone else's free tier.

## Free hosting options

### Fly.io (recommended)

`fly.toml` here points at the upstream `matchbox_server` Docker image.

```bash
cd signaling
fly auth login
fly launch --copy-config --no-deploy   # pick a unique app name
fly deploy
```

Free tier (shared-cpu-1x, 256MB, auto-stop on idle) handles dozens of
lobbies easily. Cost at low scale: $0/mo.

### Oracle Cloud (always-free ARM VM)

Spin up an Always-Free `VM.Standard.A1.Flex` (ARM, up to 4 vCPU / 24GB
RAM at no charge), install Docker, run `ghcr.io/johanhelsing/matchbox_server:latest`
behind a reverse proxy with TLS (Caddy is easiest). Permanent free; more
setup than Fly.

### Self-hosted (any VM)

```bash
docker run -d -p 3536:3536 --restart=always \
    ghcr.io/johanhelsing/matchbox_server:latest
```

You'll need TLS for browser clients (irrelevant on native), so put it
behind Caddy or nginx-with-letsencrypt.

## Pointing the client at your server

Either set the env var at build time:

```bash
SPACE_BOOSTERS_SIGNAL=wss://your-app.fly.dev cargo run
```

…or edit `DEFAULT_SIGNAL_URL` in `src/net.rs`.

## Bandwidth and cost expectations

Signaling is a few hundred bytes per peer per lobby (SDP offer/answer + ICE
candidates), then nothing. A 6-player game uses roughly 1KB total to
establish, then zero — actual gameplay traffic flows directly between
peers. You will not exceed any free tier with this game's traffic.
