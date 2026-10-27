// Web only, lazy chunk: mounts a Leaflet map with OSM tiles and one circle marker per venue.
import 'leaflet/dist/leaflet.css';
import L from 'leaflet';
import type { MapPin } from './mapPins';
import { C } from './theme';

function darkTiles() {
  if (document.getElementById('pk-leaflet-dark')) return;
  const st = document.createElement('style');
  st.id = 'pk-leaflet-dark';
  st.textContent = '.pk-tiles{filter:invert(1) hue-rotate(180deg) brightness(.82) contrast(.9)}.leaflet-container{background:#22252a;font-family:Inter_400Regular,system-ui}';
  document.head.appendChild(st);
}

export function mount(el: HTMLElement, pins: MapPin[], onPick: (venueId: string) => void) {
  darkTiles();
  const map = L.map(el, { zoomControl: true, attributionControl: true, scrollWheelZoom: false });
  L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
    maxZoom: 19, className: 'pk-tiles', attribution: '© <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
  }).addTo(map);
  const placed = pins.filter((p) => p.lat != null && p.lng != null);
  const markers = placed.map((p) => {
    const m = L.circleMarker([p.lat!, p.lng!], {
      radius: p.radius, color: C.bg, weight: 2, fillColor: p.fill, fillOpacity: p.fillOpacity,
    }).addTo(map);
    m.bindTooltip(`${p.name} · ${p.n_sets} ${p.n_sets === 1 ? 'set' : 'sets'} · ${p.heatWord}`, { direction: 'top' });
    m.on('click', () => onPick(p.venue_id));
    return m;
  });
  // paths exist once the map has a view: tag them for assistive tech (and the test) after fitBounds
  const tag = () => markers.forEach((m, i) => {
    const node = (m as any)._path as SVGElement | undefined;
    if (!node) return;
    node.setAttribute('data-testid', `pin-${placed[i].venue_id}`);
    node.setAttribute('role', 'button');
    node.setAttribute('aria-label', `${placed[i].name}, ${placed[i].heatWord}`);
  });
  if (placed.length) map.fitBounds(L.latLngBounds(placed.map((p) => [p.lat!, p.lng!] as [number, number])).pad(0.25), { maxZoom: 13 });
  else map.setView([40.73, -73.99], 11);
  tag();
  setTimeout(() => { map.invalidateSize(); tag(); }, 50);
  return () => {
    markers.forEach((m) => m.remove());
    map.remove();
  };
}
