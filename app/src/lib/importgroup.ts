// Import from Photos (UX_SPEC 4.8): which recent videos are sets, and which show were they.
// Pure functions shared by the native (expo-media-library) and web (file picker) sources.

export type Clip = {
  id: string; filename: string; creationTime: number; duration: number; // ms epoch, seconds
  lat?: number | null; lng?: number | null; uri?: string | null; file?: Blob | null; imported?: boolean;
};
export type VenuePin = { venue_id: string; name: string; lat: number; lng: number };
export type ShowGroup = { key: string; clips: Clip[]; start: number; total_s: number; venue: { name: string; venue_id: string; distance_m: number } | null };

export const SET_MIN_S = 180; // "only videos >= 3:00 by default"
export const GROUP_GAP_MS = 3 * 3600 * 1000; // same night: within 3 hours
export const GROUP_M = 500; // and 500 m

export function haversine(a: { lat: number; lng: number }, b: { lat: number; lng: number }) {
  const R = 6371000;
  const r = (d: number) => (d * Math.PI) / 180;
  const dLat = r(b.lat - a.lat);
  const dLng = r(b.lng - a.lng);
  const h = Math.sin(dLat / 2) ** 2 + Math.cos(r(a.lat)) * Math.cos(r(b.lat)) * Math.sin(dLng / 2) ** 2;
  return 2 * R * Math.asin(Math.sqrt(h));
}
const hasGeo = (c: Clip): c is Clip & { lat: number; lng: number } => typeof c.lat === 'number' && typeof c.lng === 'number';

/** The nearest known venue within 1 km of where the clip was shot (GPS → venue, shown as "The Corner Room? [AI] · 40 m away"). */
export function nearestVenue(c: Clip, venues: VenuePin[]) {
  if (!hasGeo(c) || !venues.length) return null;
  let best: { name: string; venue_id: string; distance_m: number } | null = null;
  for (const v of venues) {
    const d = haversine({ lat: c.lat, lng: c.lng }, v);
    if (d <= 1000 && (!best || d < best.distance_m)) best = { name: v.name, venue_id: v.venue_id, distance_m: Math.round(d) };
  }
  return best;
}

/** Clips from the same night (within 3 h of the previous clip, and within 500 m when both have GPS) form one show. */
export function groupShows(clips: Clip[], venues: VenuePin[] = []): ShowGroup[] {
  const sorted = [...clips].sort((a, b) => a.creationTime - b.creationTime);
  const groups: ShowGroup[] = [];
  for (const c of sorted) {
    const g = groups[groups.length - 1];
    const prev = g?.clips[g.clips.length - 1];
    const prevEnd = prev ? prev.creationTime + prev.duration * 1000 : 0;
    const near = !prev || !hasGeo(prev) || !hasGeo(c) || haversine(prev, c) <= GROUP_M;
    if (g && c.creationTime - prevEnd <= GROUP_GAP_MS && near) {
      g.clips.push(c);
      g.total_s += c.duration;
      if (!g.venue) g.venue = nearestVenue(c, venues);
    } else {
      groups.push({ key: c.id, clips: [c], start: c.creationTime, total_s: c.duration, venue: nearestVenue(c, venues) });
    }
  }
  return groups.reverse(); // newest first
}

export function splitGroup(groups: ShowGroup[], key: string): ShowGroup[] {
  return groups.flatMap((g) => (g.key !== key ? [g] : g.clips.map((c) => ({ key: c.id, clips: [c], start: c.creationTime, total_s: c.duration, venue: g.venue }))));
}

const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
/** "Fri 9:41 PM" */
export function clipTime(ms: number) {
  const d = new Date(ms);
  const h = d.getHours();
  return `${DAYS[d.getDay()]} ${((h + 11) % 12) + 1}:${String(d.getMinutes()).padStart(2, '0')} ${h < 12 ? 'AM' : 'PM'}`;
}
/** "Line up with Working set v4?" defaults on when the recording is within ±40 % of the target. */
export const alignDefault = (totalS: number, targetS?: number | null) => !!targetS && Math.abs(totalS - targetS) <= 0.4 * targetS;
