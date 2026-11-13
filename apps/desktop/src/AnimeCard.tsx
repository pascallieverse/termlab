import { useCallback, useEffect, useState } from "react";
import { Icon } from "./ui";

/** nekos.best serves safe-for-work anime art with the artist credited. */
const API = "https://nekos.best/api/v2/neko";
export const ANIME_KEY = "termlab.animeArt";

interface Art {
  url: string;
  artist: string | null;
}

/** Off unless turned on in Settings. */
export function animeArtEnabled(): boolean {
  try {
    return localStorage.getItem(ANIME_KEY) === "on";
  } catch {
    return false;
  }
}

export function setAnimeArtEnabled(on: boolean) {
  try {
    localStorage.setItem(ANIME_KEY, on ? "on" : "off");
  } catch {
    // Storage unavailable: the setting lasts for this window only.
  }
  window.dispatchEvent(new Event("termlab-anime-art"));
}

/** A random anime picture for the sidebar's empty space. Hides itself when offline. */
export function AnimeCard() {
  const [enabled, setEnabled] = useState(animeArtEnabled);
  const [art, setArt] = useState<Art | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [failed, setFailed] = useState(false);

  const next = useCallback(async () => {
    setLoaded(false);
    try {
      const r = await fetch(API);
      const data = (await r.json()) as { results?: { url: string; artist_name?: string }[] };
      const pick = data.results?.[0];
      if (!pick) throw new Error("no image");
      setArt({ url: pick.url, artist: pick.artist_name ?? null });
      setFailed(false);
    } catch {
      setFailed(true);
    }
  }, []);

  useEffect(() => {
    const onChange = () => setEnabled(animeArtEnabled());
    window.addEventListener("termlab-anime-art", onChange);
    return () => window.removeEventListener("termlab-anime-art", onChange);
  }, []);

  useEffect(() => {
    if (enabled && !art) void next();
  }, [enabled, art, next]);

  if (!enabled || failed || !art) return null;

  return (
    <figure className="anime-card">
      <button className="anime-image" onClick={() => void next()} title="Show another" aria-label="Show another picture">
        <img
          src={art.url}
          alt={art.artist ? `Anime art by ${art.artist}` : "Anime art"}
          onLoad={() => setLoaded(true)}
          onError={() => setFailed(true)}
          style={{ opacity: loaded ? 1 : 0 }}
        />
      </button>
      <figcaption>
        <span className="anime-credit">{art.artist ? `Art by ${art.artist}` : "Anime art"}</span>
        <button className="icon-btn small" aria-label="Show another picture" title="Show another" onClick={() => void next()}>
          <Icon name="refresh" />
        </button>
      </figcaption>
    </figure>
  );
}
