// Pure rules shared by the input bar and the unit test (no imports, so node can run tests/say.test.mjs on it).
export type SayKind = 'question' | 'bit';

/** The same first cut the server makes (say_api.classify), for routing a search box before the server answers:
 *  a question goes to the conversation, anything else filters the list. The server's answer is the one recorded. */
const FILLERS = /^(oh|and|so|um|uh|hey|ok|okay|also|wait|like|yeah|well|hmm|but)\s+/i;
const PREMISE = /^(what if|imagine|picture this|you know (how|when|what)|so my|my (dad|mom|wife|husband|girlfriend|boyfriend|kid|kids|son|daughter|brother|sister|roommate|neighbou?r|landlord|boss|doctor|dentist|uber|therapist|cat|dog))\b/i;
const WH = /^(what|whats|what's|when|where|which|who|whos|who's|why|how|hows|how's|do|does|did|can|could|should|would|will|is|are|was|were|have|has|had|am|any|anything|anyone|remind|list|show|find|pull|give|build|make|tell|search|look)\b/i;
const REQUEST = /^(build|make|give|show|find|pull|put|get|tell|write|new|sort|run)\b.*\b(me|up|my|a|an|the)\b/i;
const NEW_SET = /^(new|a new|build|make)\b.*\b\d+\b/i; // "new 7 for friday"
const MY = /\bmy (bits?|material|jokes?|sets?|closer|opener|notebook|stuff|best|strongest|weakest|last set|tight \d+|\d+)\b/i;
export function guessKind(text: string): SayKind {
  let t = text.trim().replace(/\s+/g, ' ');
  if (!t) return 'bit';
  if (/\?$/.test(t)) return 'question';
  for (let k = 0; k < 4; k++) t = t.replace(FILLERS, '');
  if (PREMISE.test(t)) return 'bit';
  if (WH.test(t) || REQUEST.test(t) || NEW_SET.test(t) || MY.test(t)) return 'question';
  return 'bit';
}
export const isQuestion = (t: string) => guessKind(t) === 'question';

/** The default short name, as the server cuts it (creative/notebook.short_name_of): the title before its first
 *  parenthesis, at a word break inside 18 characters. The server's value wins when a row carries one. */
export function shortNameOf(title: string, custom?: string | null): string {
  if (custom && custom.trim()) return custom.trim();
  const t = (title || '').split(' (')[0].trim() || (title || '').trim();
  if (t.length <= 18) return t;
  const cut = t.slice(0, 19);
  const sp = cut.lastIndexOf(' ');
  const out = (sp >= 4 ? cut.slice(0, sp) : t.slice(0, 18)).replace(/[\s,.;:!?-]+$/, '');
  return out || t.slice(0, 18);
}


/** Where a joke already sits in a set list (0-based), else null: the add sheet greys it and says "already at #n". */
export function positionIn(inSet: { joke_id: string; position: number }[], jokeId: string): number | null {
  const hit = inSet.find((x) => x.joke_id === jokeId);
  return hit ? hit.position : null;
}
