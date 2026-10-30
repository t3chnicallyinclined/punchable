// Map pins from /api/venues.geojson: colour = average room heat in 3 steps (one amber ramp), size = sets (1, 2–4, 5+).
import type { GeoJSON } from './creative';
import { heatStep, heatWord } from '@/rules/laughs';
import { C } from './theme';

export type MapPin = {
  venue_id: string; name: string; lat: number | null; lng: number | null; n_sets: number; heat: 0 | 1 | 2 | null; heatWord: string;
  radius: number; fill: string; fillOpacity: number; status: string;
};
const FILL = [C.mut, C.amberMid, C.amber];
export function pinsOf(g: GeoJSON | null | undefined): MapPin[] {
  return (g?.features || []).map((f) => {
    const p = f.properties;
    const h = heatStep(p.avg_room_heat);
    return {
      venue_id: p.venue_id, name: p.name, lat: f.geometry ? f.geometry.coordinates[1] : null, lng: f.geometry ? f.geometry.coordinates[0] : null,
      n_sets: p.n_sets, heat: h, heatWord: heatWord(p.avg_room_heat) || 'no read yet',
      radius: p.n_sets >= 5 ? 14 : p.n_sets >= 2 ? 11 : 8, fill: h == null ? C.borderStrong : FILL[h], fillOpacity: h === 0 ? 0.55 : 0.95, status: p.status,
    };
  });
}
