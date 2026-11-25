# Self-hosting Termlab Server

Remote access to your Claude Code, Codex, Mistral Vibe and Antigravity sessions from a
browser or your phone, on a server you run yourself.

`termlab-server` is your own server for Termlab. Your Termlab computers connect to it, and
you open it in a browser or on your phone to:

- see every session live, answer approvals, and type into a session's terminal;
- see a computer's screen and use its mouse and keyboard (macOS from Termlab 0.2.11, Windows
  and Linux from 0.2.13);
- get a push notification on your phone when a session needs you;
- send signed webhooks (Slack, CI, your own scripts) when sessions change;
- start a session when another service calls an incoming webhook URL (GitHub, CI, a form).

It's one small binary with one SQLite file. There is no shared service in between: the
only place that can reach your computers is the server you run.

**Requirements:** Termlab 0.2.7 or newer, a small Linux VPS (any x86-64 or arm64; 256 MB of
RAM is plenty), and a domain name for HTTPS, such as `remote.example.com`.

## 1. Install

The server listens on `127.0.0.1:8484` and never handles HTTPS itself: a reverse proxy on
the same machine does (step 2). Pick Docker or the binary.

### Docker

```sh
docker run -d --name termlab-server --restart unless-stopped \
  -p 127.0.0.1:8484:8484 \
  -v termlab-data:/data \
  -e TERMLAB_PUBLIC_URL=https://remote.example.com \
  ghcr.io/pascallieverse/termlab-server:latest
```

The setup token is in its log: `docker logs termlab-server`.

### Binary (systemd)

```sh
# x86-64; on arm64 use termlab-server-aarch64-unknown-linux-musl
sudo curl -L -o /usr/local/bin/termlab-server \
  https://github.com/pascallieverse/termlab/releases/latest/download/termlab-server-x86_64-unknown-linux-musl
sudo chmod +x /usr/local/bin/termlab-server
sudo useradd --system --home /var/lib/termlab-server --create-home termlab
```

The release also has `termlab-server.sha256` to check the download against.

`/etc/systemd/system/termlab-server.service`:

```ini
[Unit]
Description=Termlab Server
After=network-online.target
Wants=network-online.target

[Service]
User=termlab
Environment=TERMLAB_DATA_DIR=/var/lib/termlab-server
Environment=TERMLAB_PUBLIC_URL=https://remote.example.com
ExecStart=/usr/local/bin/termlab-server serve
Restart=on-failure
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/termlab-server
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now termlab-server
journalctl -u termlab-server   # shows the setup token
```

## 2. HTTPS with a reverse proxy

Point the domain's DNS (A and AAAA records) at the VPS first.

### Caddy (gets the certificate itself)

`/etc/caddy/Caddyfile`:

```
remote.example.com {
    reverse_proxy 127.0.0.1:8484
}
```

Caddy passes websockets through as they are; nothing else is needed.

### nginx

With a certificate from certbot:

```nginx
server {
    listen 443 ssl;
    listen [::]:443 ssl;
    http2 on;
    server_name remote.example.com;
    ssl_certificate     /etc/letsencrypt/live/remote.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/remote.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8484;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 120s;
    }
}

# in the http {} block:
map $http_upgrade $connection_upgrade { default upgrade; '' close; }
```

## 3. Create the owner account

Open `https://remote.example.com`. On the first visit it asks for:

1. **The setup token** from the server's log. Only someone who can read the log can set
   the server up.
2. **A password** of at least 12 characters.
3. **An authenticator app** (1Password, Google Authenticator, Authy, …): scan the QR code
   and enter a code to confirm.

Every sign-in needs both the password and a code from the app.

## 4. Link Termlab

1. On the server, open **Machines › Pair a machine**. It shows a code like `K7QX-2MPD`,
   valid for 10 minutes and usable once.
2. In Termlab, open **Settings › Remote access**. Enter the server's address
   (`https://remote.example.com`), the code and a name for the computer, then click
   **Link this computer**.

The computer shows up under Machines, and its sessions under Sessions. Termlab keeps the
link up from its background service, also with the window closed. Repeat this for each
computer.

On a phone, open the server in the browser and add it to the home screen. That's needed for
notifications on iPhone. Then turn on notifications in **Settings › Notifications**.

### Screen sharing

To see and control a computer's screen from the server:

1. On the computer, in Termlab, open **Settings › Remote access** and switch on **Allow
   screen sharing**. Then, depending on the system:
   - **macOS:** allow Termlab under **Screen Recording** and **Accessibility** when macOS
     asks (Termlab has buttons that open the right pages in System Settings). Without
     Accessibility the screen is view only.
   - **Windows:** nothing to allow.
   - **Linux with X11:** nothing to allow.
   - **Linux with Wayland** (the default on Ubuntu, Fedora and most current desktops): click
     **Approve…** and pick the screen in your desktop's dialog. The desktop remembers it, so
     later sessions start without anyone at the computer. The picture comes through
     GStreamer: install `gstreamer1.0-tools gstreamer1.0-pipewire` (Debian, Ubuntu) or
     `gstreamer1-plugins-base-tools pipewire-gstreamer` (Fedora).
2. Optionally switch on **Ask on this computer each time**: someone at the computer then has
   to click Allow before each session (Linux: needs `zenity` or `kdialog`).
3. On the server, click **Screen** in the header (or on the machine under Settings ›
   Machines) and enter a code from your authenticator app.

The picture is H.264, decoded by the browser (Chrome, Edge, Firefox 130+ or Safari 16.4+).
From a Mac, browsers that decode HEVC (H.265; Chrome, Edge and Safari on most computers) get
HEVC, which is sharper for the same bits; at Sharp and Native, from a Mac with Apple silicon
to a browser that decodes it (Chrome on a Mac does), HEVC 4:4:4 keeps full colour detail, so
coloured text has no colour fringes. Windows and Linux encode H.264 on the graphics card
where they can: Windows through its Media Foundation encoder (NVIDIA, Intel, AMD), Linux
through GStreamer with VA-API (Intel, AMD) or NVENC (NVIDIA): install
`gstreamer1.0-tools gstreamer1.0-plugins-bad` and your VA driver (`intel-media-va-driver`
or `mesa-va-drivers`). Without one they encode in software (Sharp at 4K runs at fewer frames
on slower processors; Balanced suits most connections); the computer's
`termlab-screen.log` says which encoder runs. Your own pointer shows the computer's pointer
shape and moves without delay (except on Linux with Wayland, where the pointer is part of
the picture). It goes through the server: while the screen
changes, count on up to 1.5 Mbit/s (Low, 1280 pixels wide, 30 fps), 6 Mbit/s (Balanced,
1920, 60 fps) or 16 Mbit/s (Sharp, 4K, 60 fps). Native (with HEVC, on displays wider than
4K) sends the full pixel size at up to 24 Mbit/s and about 24 fps. Almost nothing is sent
while the screen doesn't change. When the screen stops changing, the picture is
sent again a few times so text sharpens; the fps counter then drops, which is expected. On a phone, the screen works as a trackpad: drag to move the pointer, tap
to click, two fingers to scroll, pinch to zoom, press and hold to drag; the Keyboard button
opens the phone's keyboard, with a bar for the modifier keys, Esc, Tab and the arrows. The
**Keys** menu has shortcuts the browser would take itself (per system: ⌘ Tab on a Mac, Alt
Tab or the Start menu on Windows, Activities on Linux). Controlling a Mac from a PC (or the
other way round), Ctrl and ⌘ swap places; the button next to it turns that off.

The computer stays awake and doesn't lock while a session runs. A computer that is already
locked, at the login window or asleep can't be shared yet. On Windows, programs running as
administrator and UAC prompts can't be controlled (Windows doesn't allow it). Clipboard
sharing on Linux uses `wl-copy`/`wl-paste` (Wayland) or `xclip`/`xsel` (X11).

macOS remembers the two permissions per app build. A Termlab built without a Developer ID
signature asks again after each update.

## Settings

Every setting is a flag or an environment variable.

| Flag | Environment | Default | What it does |
|---|---|---|---|
| `--public-url` | `TERMLAB_PUBLIC_URL` | (required) | The address people and Termlab use. `https://` makes cookies Secure. |
| `--listen` | `TERMLAB_LISTEN` | `127.0.0.1:8484` | Where the server listens. Keep it on localhost (Docker: `0.0.0.0` inside the container, published on `127.0.0.1`). |
| `--data-dir` | `TERMLAB_DATA_DIR` | `./termlab-data` | The database and keys. |
| `--server-name` | `TERMLAB_SERVER_NAME` | `Termlab Server` | Shown in the web UI and in Termlab. |
| `--trust-proxy` | `TERMLAB_TRUST_PROXY` | `true` | Use `X-Forwarded-For` from a proxy on this machine or a private network (for sign-in limits and the audit log). |
| `--allow-private-webhooks` | `TERMLAB_ALLOW_PRIVATE_WEBHOOKS` | `false` | Let webhooks reach private and loopback addresses, and plain `http://`. |

## Security

- **One owner per server.** Sign-in needs the password (argon2id) and an authenticator code.
  After 5 failures from an address, it's blocked for 15 minutes.
- **Sessions:** HttpOnly, SameSite=Strict cookies, valid for 30 days. Every request that
  changes something also needs a custom header. The websocket only accepts the server's own
  origin.
- **Machines:**
  - Each computer has its own token, which the server stores only as a hash.
  - Removing a computer under Machines cuts it off immediately, and Termlab unlinks itself.
  - Unlinking in Termlab removes it from the server.
- **The audit log** (Settings) records sign-ins, failed sign-ins, every approval, message,
  stop and hold, sessions started (from the web or an incoming webhook), and pairing and
  webhook changes, with the address they came from.
- **Screen sharing** is off until it's switched on in Termlab on that computer; the server
  can't switch it on. Each session needs an authenticator code (again after 10 minutes), and
  can need someone at the computer to allow it. macOS shows its screen-recording indicator
  while it runs, and Termlab's settings show who is connected, with a Stop button.
  Switching it off ends a running session. The server relays the picture and input without
  storing them. The audit log records each session's start and end and wrong codes.
- **Terminal data:** Termlab only streams a terminal while you have it open, and the server
  passes it through without storing it. What the server stores is session status, the
  last few screen lines, and webhook delivery logs.
- **Webhooks:** they refuse private, loopback and link-local addresses, unless you turn that
  on. Each delivery is signed:

  ```
  X-Termlab-Signature: t=<unix time>,v1=<hex HMAC-SHA256 of "<t>.<body>">
  ```

  To check it, compute the HMAC with the endpoint's secret over the raw body, compare in
  constant time, and reject timestamps older than 5 minutes. An endpoint can also send a
  bearer token, basic auth or an API-key header, and headers of your own. Those secrets are
  stored in the database and never shown again in the web UI.
- **Incoming webhooks:** the URL (`/api/hooks/<token>`) is the only credential, so treat it
  like a password; Settings › Webhooks can replace it. Anyone holding it can start a
  session with a message of their choosing, with the agent and permissions the hook fixes,
  in its project or, when the hook takes the project from the request, in any project on
  that computer. Labels named by the request are only made when the hook allows it. Prefer *Ask first* or *Auto-edit*: with *Full auto* the caller can have the agent
  run commands on that computer. Each hook takes 20 calls a minute, and every call goes to
  the audit log (`session.start`, with the caller's address).

## Backups and upgrades

- **Back up** the data directory: `termlab-server.sqlite` (plus its `-wal` file) and
  `vapid.key`, the key for phone notifications. With Docker, that's the `termlab-data`
  volume. To copy the database while the server runs, use
  `sqlite3 termlab-server.sqlite ".backup backup.sqlite"`.
- **Upgrade with Docker:** `docker pull ghcr.io/pascallieverse/termlab-server:latest`, then
  recreate the container with the same volume.
- **Upgrade the binary:** download the new one over the old one, then
  `sudo systemctl restart termlab-server`. Paired computers reconnect by themselves.

## Lost your password or authenticator

On the server:

```sh
termlab-server reset-owner --data-dir /var/lib/termlab-server   # binary
docker exec termlab-server /termlab-server reset-owner          # Docker
```

This removes the owner account (paired computers, webhooks and logs stay). Restart the
server and it prints a new setup token.

## API

The web UI and Termlab use the API in [server-protocol.md](server-protocol.md). Scripts can use
it too, with a signed-in session.
