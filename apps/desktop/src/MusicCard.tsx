import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { api, type YouTubeMusicState } from "./api";
import { TruncatedText } from "./TruncatedText";
import { Icon } from "./ui";

// Music for the sidebar, two ways:
//  - YouTube Music itself, docked: the app places a separate web view (music.youtube.com,
//    where you sign in) over a slot in this card. It can't be an iframe: YouTube Music
//    refuses to be framed. The page has no access to Termlab.
//  - Stations, through YouTube's embeddable player (privacy-enhanced
//    domain), driven with the player's postMessage protocol so no YouTube script runs in
//    Termlab's window.

const YT_ORIGIN = "https://www.youtube-nocookie.com";
const ENABLED_KEY = "termlab.music";
const SOURCE_KEY = "termlab.music.source";
const OPEN_KEY = "termlab.music.open";
const VOLUME_KEY = "termlab.music.volume";
const STATION_KEY = "termlab.music.station";
/** The docked page is drawn at this scale: YouTube Music's mobile layout, laid out about
 *  1.6x wider than the slot (roughly a phone's width) and shrunk to fit. */
const DOCK_ZOOM = 0.62;
/** After a station starts, YouTube Music still playing means "not paused yet", not "you pressed play there". */
const STATION_SETTLE_MS = 4000;

type MusicTab = "ytm" | "stations";

/**
 * The docked page is a native view drawn above Termlab's own interface, so it must step aside
 * for anything of Termlab's that would be on top of it: any dialog or the command palette,
 * and menus or tooltips that overlap it.
 */
function isCovered(slot: DOMRect): boolean {
  if (document.querySelector('[role="dialog"]:not([data-ytm-host]), #pal-list')) return true;
  for (const el of document.querySelectorAll('[role="menu"], [role="tooltip"]')) {
    const r = el.getBoundingClientRect();
    if (r.width > 0 && r.left < slot.right && r.right > slot.left && r.top < slot.bottom && r.bottom > slot.top) {
      return true;
    }
  }
  return false;
}
export const MUSIC_EVENT = "termlab-music";

export type MusicSource =
  | {
      kind: "video";
      video: string;
      list?: string;
      label: string;
      /** For stations: the channel whose current live stream replaces a video that stopped. */
      channel?: string;
    }
  | { kind: "playlist"; list: string; label: string };

const LOFI_GIRL = "UCSJ4gkVC6NrvII8umztf0Ow";

/** Stations that play without a link: 24/7 live streams, checked to play in embeds. */
export const STATIONS: { source: MusicSource; hint: string }[] = [
  { source: { kind: "video", video: "rFZHOHl-L8A", channel: LOFI_GIRL, label: "Lofi Girl" }, hint: "lofi hip hop radio, beats to relax/study to" },
  { source: { kind: "video", video: "7NOSDKb0HlU", channel: "UCOxqgCwgOqC2lMqC5PYz_Dg", label: "Chillhop" }, hint: "Chillhop's lofi hip hop radio" },
  { source: { kind: "video", video: "Dx5qFachd3A", channel: "UCJhjE7wbdYAae1G25m0tHAA", label: "Jazz café" }, hint: "relaxing jazz piano radio" },
  { source: { kind: "video", video: "4xDzrJKXOOY", channel: LOFI_GIRL, label: "Synthwave" }, hint: "synthwave radio, beats to chill/game to" },
  { source: { kind: "video", video: "VAlMDl00mYY", channel: LOFI_GIRL, label: "Sleep" }, hint: "sleep lofi radio, relaxing music to sleep to" },
];

// ---- settings ----

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function write(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage unavailable: the choice lasts for this window only.
  }
}

/** Off unless turned on in Settings. */
export function musicEnabled(): boolean {
  return read(ENABLED_KEY) === "on";
}

export function setMusicEnabled(on: boolean) {
  write(ENABLED_KEY, on ? "on" : "off");
  window.dispatchEvent(new Event(MUSIC_EVENT));
}

function savedSource(): MusicSource | null {
  try {
    const s = JSON.parse(read(SOURCE_KEY) ?? "null") as MusicSource | null;
    return s && typeof s === "object" && "kind" in s ? s : null;
  } catch {
    return null;
  }
}

const VIDEO_ID = /^[A-Za-z0-9_-]{11}$/;

function embedUrl(source: MusicSource): string {
  const params = new URLSearchParams({
    enablejsapi: "1",
    autoplay: "1",
    playsinline: "1",
    rel: "0",
    iv_load_policy: "3",
    origin: window.location.origin,
  });
  switch (source.kind) {
    case "video":
      if (source.list) params.set("list", source.list);
      return `${YT_ORIGIN}/embed/${source.video}?${params}`;
    case "playlist":
      params.set("list", source.list);
      return `${YT_ORIGIN}/embed/videoseries?${params}`;
  }
}

const ERRORS: Record<number, string> = {
  2: "That link doesn't point to a playable video.",
  5: "This video can't be played here.",
  100: "That video was removed or is private.",
  101: "Its owner doesn't allow playing it outside YouTube.",
  150: "Its owner doesn't allow playing it outside YouTube.",
  153: "YouTube refused to play inside Termlab.",
};

// ---- the card ----

/** YouTube player states, from the player API. */
const PLAYING = 1;
const BUFFERING = 3;

export function MusicCard() {
  const [enabled, setEnabled] = useState(musicEnabled);
  const [source, setSource] = useState<MusicSource | null>(savedSource);
  const [active, setActive] = useState(false); // the player is loaded (and audible)
  const [open, setOpen] = useState(() => read(OPEN_KEY) === "1");
  const [state, setState] = useState(-1);
  const [now, setNow] = useState<{ title: string; author: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [volume, setVolume] = useState(() => Number(read(VOLUME_KEY) ?? 70));
  /** The volume last set here for YouTube Music (its own report lags a moment behind). */
  const [ytmVolume, setYtmVolume] = useState<number | null>(null);
  const [volumeOpen, setVolumeOpen] = useState(false);
  /** The computer's output volume; null where it can't be read (then the player's is used). */
  const [macVolume, setMacVolume] = useState<number | null>(null);
  const loadMacVolume = useCallback(() => {
    void api.systemVolume().then(setMacVolume, () => setMacVolume(null));
  }, []);
  // It also changes from the keyboard and the menu bar: re-read when the app or slider is used.
  useEffect(() => {
    loadMacVolume();
    window.addEventListener("focus", loadMacVolume);
    return () => window.removeEventListener("focus", loadMacVolume);
  }, [loadMacVolume]);
  useEffect(() => {
    if (volumeOpen) loadMacVolume();
  }, [volumeOpen, loadMacVolume]);
  // While dragging, at most one change every 70 ms reaches the system, always ending on the last.
  const macVolumeSend = useRef<{ timer?: ReturnType<typeof setTimeout>; next?: number }>({});
  const sendMacVolume = (v: number) => {
    const state = macVolumeSend.current;
    state.next = v;
    if (state.timer) return;
    const flush = () => {
      const value = state.next;
      state.next = undefined;
      state.timer = undefined;
      if (value === undefined) return;
      void api.setSystemVolume(value).catch(() => {});
      state.timer = setTimeout(flush, 70);
    };
    flush();
  };
  const volumeRoot = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!volumeOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!volumeRoot.current?.contains(e.target as Node)) setVolumeOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setVolumeOpen(false);
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [volumeOpen]);
  const frame = useRef<HTMLIFrameElement>(null);
  const ready = useRef(false);
  /** A station whose stream stopped is retried once with the channel's current stream. */
  const retried = useRef(false);
  const sourceRef = useRef(source);
  sourceRef.current = source;
  /** What Termlab's YouTube Music window is playing (null: not open, or nothing played yet). */
  const [ytm, setYtm] = useState<YouTubeMusicState | null>(null);
  const activeRef = useRef(active);
  activeRef.current = active;
  /** When a station last started or resumed: YouTube Music is being paused for it. */
  const embedStartedAt = useRef(0);
  /** Whether YouTube Music was playing at the previous check. */
  const ytmWasPlaying = useRef(false);

  // Follow the YouTube Music view. Only one thing plays at a time, and the newest wins:
  //  - a station was just started: YouTube Music is told to pause until it has;
  //  - YouTube Music starts while a station plays (you pressed play there): the station stops.
  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    const poll = () =>
      api.youtubeMusicState().then(
        (s) => {
          if (cancelled) return;
          setYtm(s);
          const playing = !!s?.playing;
          if (playing && activeRef.current) {
            const stationIsNewer =
              Date.now() - embedStartedAt.current < STATION_SETTLE_MS || ytmWasPlaying.current;
            if (stationIsNewer) {
              // It hasn't paused yet (the page was busy, or moved on to its next song).
              void api.youtubeMusicControl("pause").catch(() => {});
            } else {
              setActive(false);
              setState(-1);
              setNow(null);
            }
          }
          // You pressed play in YouTube Music: that's the player in use now.
          if (playing && !ytmWasPlaying.current && !activeRef.current) {
            setTabState("ytm");
          }
          ytmWasPlaying.current = playing;
        },
        () => {},
      );
    void poll();
    const timer = setInterval(poll, 1000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [enabled]);

  const ytmControl = (action: "playPause" | "next" | "previous") => {
    if (action === "playPause") setYtm((s) => s && { ...s, playing: !s.playing });
    void api.youtubeMusicControl(action).catch(() => {});
  };

  // Which player the card is on: YouTube Music or the stations. It is the tab that's open,
  // and it follows what was last played, so the collapsed bar and the expanded card agree.
  // Stations is where the card starts every time the app opens.
  const [tab, setTabState] = useState<MusicTab>("stations");
  const setTab = (t: MusicTab) => {
    setTabState(t);
  };
  /** YouTube Music enlarged over the app (for signing in or browsing). */
  const [big, setBig] = useState(false);
  const [covered, setCovered] = useState(false);
  const placement = useRef<Promise<unknown>>(Promise.resolve());
  const dock = useRef<HTMLDivElement>(null);
  const bigDock = useRef<HTMLDivElement>(null);
  const showYtm = enabled && (big || (open && tab === "ytm"));

  // Serialize native operations so stale dock requests cannot cover a menu or a closed card.
  useEffect(() => {
    let cancelled = false;
    let revision = 0;
    let last = "";
    const place = () => {
      const el = big ? bigDock.current : dock.current;
      const r = el?.getBoundingClientRect();
      const hide = !showYtm || !r || isCovered(r) || r.width < 40 || r.height < 40
        || r.top < 0 || r.left < 0 || r.bottom > window.innerHeight || r.right > window.innerWidth;
      const key = hide ? "hidden" : [r.left, r.top, r.width, r.height].map(Math.round).join(",");
      if (key === last) return;
      last = key;
      const current = ++revision;
      setCovered(!!hide);
      placement.current = placement.current.catch(() => {}).then(async () => {
        if (cancelled || current !== revision) return;
        try {
          if (hide || !r) await api.youtubeMusicHide();
          else await api.youtubeMusicDock(
            { x: r.left, y: r.top, width: r.width, height: r.height },
            big ? 1 : DOCK_ZOOM,
          );
        } catch (e) {
          if (!cancelled && current === revision) {
            last = ""; // Allow a later placement to retry.
            setError(typeof e === "string" ? e : "Couldn't open YouTube Music.");
          }
        }
      });
    };
    place();
    const observer = new ResizeObserver(place);
    const el = big ? bigDock.current : dock.current;
    if (el) observer.observe(el);
    // Position and covering menus can change without resizing the slot.
    const timer = setInterval(place, 120);
    window.addEventListener("resize", place);
    return () => {
      cancelled = true;
      observer.disconnect();
      clearInterval(timer);
      window.removeEventListener("resize", place);
    };
  }, [showYtm, big]);
  useEffect(() => () => {
    placement.current = placement.current.catch(() => {}).then(() => api.youtubeMusicHide()).catch(() => {});
  }, []);

  useEffect(() => {
    if (!big) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setBig(false);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [big]);

  const showYouTubeMusic = () => {
    setTab("ytm");
    if (!open) toggleOpen();
  };

  useEffect(() => {
    const on = () => setEnabled(musicEnabled());
    window.addEventListener(MUSIC_EVENT, on);
    return () => window.removeEventListener(MUSIC_EVENT, on);
  }, []);

  const post = useCallback((msg: object) => {
    frame.current?.contentWindow?.postMessage(JSON.stringify(msg), YT_ORIGIN);
  }, []);
  const command = useCallback(
    (func: string, args: unknown[] = []) => post({ event: "command", func, args, id: 1, channel: "widget" }),
    [post],
  );

  // Player → card: state, the current song and errors.
  useEffect(() => {
    function onMessage(e: MessageEvent) {
      if (e.origin !== YT_ORIGIN || e.source !== frame.current?.contentWindow) return;
      let data: { event?: string; info?: unknown };
      try {
        data = typeof e.data === "string" ? JSON.parse(e.data) : e.data;
      } catch {
        return;
      }
      if (data.event === "onReady" || data.event === "initialDelivery") {
        if (!ready.current) {
          ready.current = true;
          command("setVolume", [volume]);
        }
      }
      if (data.event === "onError") {
        const s = sourceRef.current;
        if (s?.kind === "video" && s.channel && !retried.current) {
          retried.current = true;
          setError("This stream ended; finding the channel's current one…");
          void api.youtubeLiveVideo(s.channel).then(
            (video) => {
              if (video && video !== s.video) replaceSource({ ...s, video });
              else setError("This station isn't streaming right now.");
            },
            () => setError("This station isn't streaming right now."),
          );
          return;
        }
        setError(ERRORS[Number(data.info)] ?? "YouTube couldn't play this.");
        return;
      }
      if (data.event === "onStateChange" && typeof data.info === "number") setState(data.info);
      const info = data.info as
        | { playerState?: number; videoData?: { title?: string; author?: string } }
        | undefined;
      if (info && typeof info === "object") {
        if (typeof info.playerState === "number") {
          setState(info.playerState);
          if (info.playerState === PLAYING) setError(null);
        }
        const title = info.videoData?.title;
        if (title) setNow({ title, author: info.videoData?.author ?? "" });
      }
    }
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [command, volume]);

  // Ask the player for events until it answers (it ignores messages before it's loaded).
  const onFrameLoad = useCallback(() => {
    ready.current = false;
    let tries = 0;
    const timer = setInterval(() => {
      if (ready.current || ++tries > 40) return clearInterval(timer);
      post({ event: "listening", id: 1, channel: "widget" });
    }, 250);
  }, [post]);

  // A station's replacement stream: remembered, so it plays straight away next time.
  function replaceSource(next: MusicSource) {
    setSource(next);
    write(SOURCE_KEY, JSON.stringify(next));
    write(`${STATION_KEY}.${next.label}`, next.kind === "video" ? next.video : "");
    setError(null);
  }

  function play(next: MusicSource) {
    retried.current = false;
    // A station plays the stream that last worked for it.
    const known = next.kind === "video" && next.channel ? read(`${STATION_KEY}.${next.label}`) : null;
    if (known && next.kind === "video" && VIDEO_ID.test(known)) next = { ...next, video: known };
    setSource(next);
    write(SOURCE_KEY, JSON.stringify(next));
    setNow(null);
    setError(null);
    setState(-1);
    embedStartedAt.current = Date.now();
    void api.youtubeMusicControl("pause").catch(() => {});
    setActive(true);
    setTab("stations");
  }

  function toggleOpen() {
    setOpen((o) => {
      write(OPEN_KEY, o ? "0" : "1");
      return !o;
    });
  }

  function playPause() {
    if (!active && ytm) return ytmControl("playPause");
    // Nothing to resume on this player (YouTube Music not loaded yet, or no station picked):
    // open the card to choose.
    if (!active && (tab === "ytm" || !source)) {
      if (!open) toggleOpen();
      return;
    }
    if (!active) {
      embedStartedAt.current = Date.now();
      void api.youtubeMusicControl("pause").catch(() => {});
      return setActive(true);
    }
    if (state === PLAYING || state === BUFFERING) return command("pauseVideo");
    // Resuming a paused station also takes over from YouTube Music.
    embedStartedAt.current = Date.now();
    void api.youtubeMusicControl("pause").catch(() => {});
    command("playVideo");
  }

  function stop() {
    setActive(false);
    setState(-1);
    setNow(null);
  }

  if (!enabled) return null;

  // The YouTube Music window drives the card unless a station is playing here.
  const fromYtm = !active && ytm !== null;
  const playing = fromYtm ? ytm.playing : active && (state === PLAYING || state === BUFFERING);
  const canSkip =
    fromYtm || (active && (source?.kind === "playlist" || (source?.kind === "video" && !!source.list)));
  // Nothing playing: the bar names the player the card will open on.
  const resumable = tab === "stations" && source !== null;
  const title = fromYtm
    ? ytm.title
    : active
      ? (now?.title ?? source?.label ?? "")
      : resumable
        ? source.label
        : tab === "ytm"
          ? "YouTube Music"
          : "Stations";
  const subtitle = fromYtm
    ? ytm.artist || "YouTube Music"
    : active
      ? (now?.author ?? "Loading…")
      : resumable
        ? "Press play to resume"
        : tab === "ytm"
          ? "Open to play"
          : "Pick a station";
  // One volume control for whichever is playing: YouTube Music's own player, or the station.
  // The slider sets the computer's volume where that can be read (macOS); otherwise the
  // player's own.
  const shownVolume = Math.round(macVolume ?? (fromYtm ? (ytmVolume ?? ytm.volume ?? 100) : volume));
  const changeVolume = (v: number) => {
    if (macVolume !== null) {
      setMacVolume(v);
      sendMacVolume(v);
    } else if (fromYtm) {
      setYtmVolume(v);
      void api.youtubeMusicVolume(v).catch(() => {});
    } else {
      setVolume(v);
      write(VOLUME_KEY, String(v));
      command("setVolume", [v]);
    }
  };
  const skip = (dir: "next" | "previous") =>
    fromYtm ? ytmControl(dir) : command(dir === "next" ? "nextVideo" : "previousVideo");

  return (
    <section
      className={`music-card${open ? " open" : ""}${active ? " active" : ""}${fromYtm ? " ytm" : ""}`}
      aria-label="Music"
    >
      <div className="music-player">
        {fromYtm ? (
          <button className="music-cover" onClick={showYouTubeMusic} aria-label="Show YouTube Music">
            {ytm.artwork ? <img src={ytm.artwork} alt="" /> : <Icon name="music" size={16} />}
          </button>
        ) : active && source ? (
          <iframe
            ref={frame}
            key={embedUrl(source)}
            src={embedUrl(source)}
            title={`YouTube player: ${now?.title ?? source.label}`}
            allow="autoplay; encrypted-media; picture-in-picture"
            referrerPolicy="strict-origin-when-cross-origin"
            onLoad={onFrameLoad}
          />
        ) : (
          <button className="music-placeholder" onClick={playPause} aria-label="Open the music player">
            <Icon name="music" size={16} />
          </button>
        )}
      </div>

      <div className="music-bar">
        <div className="music-now">
          <TruncatedText className="music-title" text={title} />
          <span className="music-sub">{error ?? subtitle}</span>
        </div>
        <div className="music-controls">
          {/* The collapsed bar leaves "Previous" out, to keep room for the song's name. */}
          {canSkip && open && (
            <button className="icon-btn small" aria-label="Previous" onClick={() => skip("previous")}>
              <Icon name="prev" />
            </button>
          )}
          <button
            className="icon-btn small"
            aria-label={playing ? "Pause" : "Play"}
            onClick={playPause}
            disabled={!fromYtm && !active && !resumable && open}
          >
            <Icon name={playing ? "pause" : "play"} />
          </button>
          {canSkip && (
            <button className="icon-btn small" aria-label="Next" onClick={() => skip("next")}>
              <Icon name="next" />
            </button>
          )}
          {(fromYtm || active) && (
            <div className="music-vol" ref={volumeRoot}>
              <button
                className="icon-btn small"
                aria-label={macVolume !== null ? "Mac volume" : "Volume"}
                aria-expanded={volumeOpen}
                title={`${macVolume !== null ? "Mac volume" : "Volume"} ${shownVolume}%`}
                onClick={() => setVolumeOpen((o) => !o)}
              >
                <Icon name={shownVolume === 0 ? "volumeOff" : "volume"} />
              </button>
              {volumeOpen && (
                <div className="music-vol-pop" role="group" aria-label={macVolume !== null ? "Mac volume" : "Volume"}>
                  <input
                    type="range"
                    min={0}
                    max={100}
                    aria-label={macVolume !== null ? "Mac volume" : "Volume"}
                    value={shownVolume}
                    onChange={(e) => changeVolume(Number(e.target.value))}
                  />
                  <span className="music-vol-value">{shownVolume}%</span>
                </div>
              )}
            </div>
          )}
          <button
            className="icon-btn small music-expand"
            aria-label={open ? "Collapse the music player" : "Expand the music player"}
            aria-expanded={open}
            onClick={toggleOpen}
          >
            <Icon name="chevron" />
          </button>
        </div>
      </div>

      {open && (
        <div className="music-body">
          <div className="music-tabs">
            <div role="tablist" aria-label="Music source" className="segmented small">
              <button
                role="tab"
                aria-selected={tab === "stations"}
                className={tab === "stations" ? "on" : ""}
                onClick={() => setTab("stations")}
              >
                Stations
              </button>
              <button role="tab" aria-selected={tab === "ytm"} className={tab === "ytm" ? "on" : ""} onClick={() => setTab("ytm")}>
                YT Music
              </button>
            </div>
            {tab === "ytm" && <YtmNav />}
          </div>
          {tab === "ytm" ? (
            <>
              <div className="ytm-dock">
                <div className="ytm-dock-slot" ref={dock}>
                  <span className="muted small">
                    {big ? "Shown enlarged" : covered ? "Player temporarily hidden" : "Loading YouTube Music…"}
                  </span>
                </div>
              </div>
              <button className="btn small music-browse" onClick={() => setBig(true)}>
                <Icon name="expand" size={12} /> Browse YouTube Music
              </button>
            </>
          ) : (
            <>
            {active && (
              <div className="music-volume">
                <label htmlFor="music-vol" className="sr">
                  Volume
                </label>
                <input
                  id="music-vol"
                  type="range"
                  min={0}
                  max={100}
                  value={volume}
                  onChange={(e) => {
                    const v = Number(e.target.value);
                    setVolume(v);
                    write(VOLUME_KEY, String(v));
                    command("setVolume", [v]);
                  }}
                />
                <button className="btn small" onClick={stop}>
                  Stop
                </button>
              </div>
            )}
            <div className="music-stations" role="group" aria-label="Stations">
              {STATIONS.map(({ source: st, hint }) => {
                const on = active && source?.label === st.label;
                return (
                  <button
                    key={st.label}
                    className={`chip${on ? " on" : ""}`}
                    aria-pressed={on}
                    title={hint}
                    onClick={() => play(st)}
                  >
                    {st.label}
                  </button>
                );
              })}
            </div>
            </>
          )}
        </div>
      )}

      {big &&
        createPortal(
          <div className="ytm-overlay" onMouseDown={(e) => e.target === e.currentTarget && setBig(false)}>
            <div className="ytm-big" role="dialog" aria-label="YouTube Music" data-ytm-host>
              <div className="ytm-big-head">
                <strong>YouTube Music</strong>
                <YtmNav />
                <div className="spacer" />
                <button
                  className="icon-btn small"
                  aria-label="Back to the sidebar"
                  title="Back to the sidebar (the music keeps playing)"
                  onClick={() => setBig(false)}
                >
                  <Icon name="close" />
                </button>
              </div>
              <div className="ytm-big-slot" ref={bigDock} />
            </div>
          </div>,
          document.body,
        )}
    </section>
  );
}

/** Back, home and reload for the docked YouTube Music page (it has no browser bar of its own). */
function YtmNav() {
  const go = (action: "back" | "home" | "reload") => void api.youtubeMusicNavigate(action).catch(() => {});
  return (
    <div className="ytm-nav" role="group" aria-label="YouTube Music page">
      <button className="icon-btn small ytm-nav-back" aria-label="Back" title="Back" onClick={() => go("back")}>
        <Icon name="arrow" />
      </button>
      <button
        className="icon-btn small"
        aria-label="YouTube Music home"
        title="YouTube Music home"
        onClick={() => go("home")}
      >
        <Icon name="music" />
      </button>
      <button className="icon-btn small" aria-label="Reload" title="Reload" onClick={() => go("reload")}>
        <Icon name="refresh" />
      </button>
    </div>
  );
}
