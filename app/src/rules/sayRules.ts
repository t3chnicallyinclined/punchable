// Pure rules shared by the input bar and the unit test (no imports, so node can run tests/say.test.mjs on it).
export type SayKind = 'question' | 'bit' | 'command';

/** The same first cut the server makes (say_api.classify), for routing a search box before the server answers:
 *  a question or a command goes to the conversation, anything else filters the list. The server's answer is the
 *  one recorded. */
const FILLERS = /^(oh|and|so|um|uh|hey|ok|okay|also|wait|like|yeah|well|hmm|but)\s+/i;
const PREMISE = /^(what if|imagine|picture this|you know (how|when|what)|so my|my (dad|mom|wife|husband|girlfriend|boyfriend|kid|kids|son|daughter|brother|sister|roommate|neighbou?r|landlord|boss|doctor|dentist|uber|therapist|cat|dog))\b/i;
const WH = /^(what|whats|what's|when|where|which|who|whos|who's|why|how|hows|how's|do|does|did|can|could|should|would|will|is|are|was|were|have|has|had|am|any|anything|anyone|remind|list|show|find|pull|give|build|make|tell|search|look)\b/i;
const REQUEST = /^(build|make|give|show|find|pull|put|get|tell|write|new|sort|run)\b.*\b(me|up|my|a|an|the)\b/i;
const NEW_SET = /^(new|a new|build|make)\b.*\b\d+\b/i; // "new 7 for friday"
const MY = /\bmy (bits?|material|jokes?|sets?|closer|opener|notebook|stuff|best|strongest|weakest|last set|tight \d+|\d+)\b/i;

// Commands (the server's say_api.parse_command is the one that acts; this mirror only routes the bar).
const NUMS = 'one|two|three|four|five|six|seven|eight|nine|ten|twelve|fifteen|twenty|thirty|forty|forty-five|forty five|sixty';
const NEW_SET_CMD = new RegExp(
  `^(?:(?:make|build|start|create|give|set up|setup)(?: me)?|new|a new)\\s+(?:(?:a|an|another)\\s+)?(?:(?:new|tight|quick|fresh|clean|solid|killer)\\s+)*(\\d{1,3}|${NUMS})(?:\\s*(?:min|mins|minute|minutes|minuter|m))?(?:\\s+(?:set|set list|setlist|list|one|spot))?(?:\\s+(?:for|at|on)\\s+(.+?))?\\s*$`,
  'i',
);
const ADD_CMD = /^(?:add|put|stick|throw|drop|slot)\s+(?:in\s+)?(?:the\s+)?(.+?)\s+(?:in|into|on|onto|to)\s+(?:tonight(?:'s|s)?(?:\s+(?:set\s+list|setlist|set|list))?|(?:the|my)\s+(?:set\s+list|setlist|set|list)(?:\s+(?:for\s+)?tonight)?)\s*$/i;
const NUM_WORDS: Record<string, number> = { one: 1, two: 2, three: 3, four: 4, five: 5, six: 6, seven: 7, eight: 8, nine: 9, ten: 10, twelve: 12, fifteen: 15, twenty: 20, thirty: 30, forty: 40, 'forty-five': 45, 'forty five': 45, sixty: 60 };

export type Command = { op: 'new_set'; minutes: number; title?: string } | { op: 'add_to_tonight'; query: string };
export function parseCommand(text: string): Command | null {
  let t = text.trim().replace(/\s+/g, ' ').replace(/’/g, "'").replace(/[.! ]+$/, '');
  if (!t || /\?$/.test(t)) return null;
  for (let k = 0; k < 4; k++) t = t.replace(FILLERS, '');
  const m = NEW_SET_CMD.exec(t);
  if (m) {
    const n = /^\d+$/.test(m[1]) ? Number(m[1]) : NUM_WORDS[m[1].toLowerCase()];
    if (!n || n < 1 || n > 240) return null;
    const target = (m[2] || '').replace(/^(the|a|an)\s+/i, '').trim();
    return { op: 'new_set', minutes: n, title: target ? `${target[0].toUpperCase()}${target.slice(1)} · ${n}` : undefined };
  }
  const a = ADD_CMD.exec(t);
  if (a) return { op: 'add_to_tonight', query: a[1].trim() };
  return null;
}

export function guessKind(text: string): SayKind {
  let t = text.trim().replace(/\s+/g, ' ');
  if (!t) return 'bit';
  if (/\?$/.test(t)) return 'question';
  for (let k = 0; k < 4; k++) t = t.replace(FILLERS, '');
  if (PREMISE.test(t)) return 'bit';
  if (parseCommand(t)) return 'command';
  if (WH.test(t) || REQUEST.test(t) || NEW_SET.test(t) || MY.test(t)) return 'question';
  return 'bit';
}
export const isQuestion = (t: string) => guessKind(t) !== 'bit';

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
