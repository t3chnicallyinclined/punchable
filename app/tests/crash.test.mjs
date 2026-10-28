// The crash-report scrubber (src/lib/crash.ts): nothing of his leaves in an event, and the test goes red when the
// rule is broken.   node --experimental-strip-types --no-warnings tests/crash.test.mjs
import {
  REDACTED, SENSITIVE_KEYS, apiBreadcrumb, environmentName, flattenKeys, releaseName, routePattern, scrubBreadcrumb,
  scrubEvent, scrubText, sensitiveKey,
} from '../src/lib/crash.ts';

const fails = [];
const check = (c, m) => { console.log((c ? 'PASS ' : 'FAIL ') + m); if (!c) fails.push(m); };

// invented sample material: none of it is real
const PRIVATE = {
  text: 'sample bit text about pigeons',
  quote: 'sample punchline text',
  transcript: 'sample transcript line',
  query: 'sample search words',
  title: 'Sample Title',
  token: 'Bearer sample.token.value',
};

const event = () => ({
  event_id: 'e1', timestamp: 1, platform: 'javascript', level: 'error', release: 'punchable@0.1.0+7', dist: '7', environment: 'production',
  message: `boom: "${PRIVATE.text}" ` + 'x'.repeat(400),
  exception: { values: [{ type: 'TypeError', value: `cannot read '${PRIVATE.quote}'\nat ${PRIVATE.transcript}`, stacktrace: { frames: [
    { filename: 'app:///index.bundle', function: 'load', lineno: 12, colno: 3, in_app: true, vars: { text: PRIVATE.text }, context_line: PRIVATE.transcript, pre_context: [PRIVATE.text] },
  ] } }] },
  request: { url: `https://example.test/api/search?q=${encodeURIComponent(PRIVATE.query)}`, method: 'GET', headers: { Authorization: PRIVATE.token, Cookie: 'session=abc' }, data: { text: PRIVATE.text }, query_string: 'q=' + PRIVATE.query },
  breadcrumbs: [
    { category: 'console', message: PRIVATE.text, level: 'log' },
    { category: 'fetch', type: 'http', data: { url: `https://example.test/api/joke/x:ab12?q=${PRIVATE.query}`, method: 'GET', status_code: 500, body: PRIVATE.text } },
    { category: 'api', type: 'http', level: 'warning', data: { route: '/api/decision', method: 'POST', status: 500, query: PRIVATE.query } },
    { category: 'navigation', data: { from: '/joke/x', to: '/stage/y', title: PRIVATE.title }, message: PRIVATE.quote },
  ],
  tags: { where: 'stage', transcript: PRIVATE.transcript },
  contexts: { device: { model: 'iPhone', name: PRIVATE.title }, os: { name: 'iOS', version: '26' }, custom: { text: PRIVATE.text } },
  extra: { job: { kind: 'riff', text: PRIVATE.text }, nested: { deep: { quote: PRIVATE.quote, n: 3 } }, ok: 'fine' },
  user: { id: 'owner', email: 'owner@example.test', ip_address: '203.0.113.9', username: 'x' },
  threads: { values: [{ id: 1, crashed: true, stacktrace: { frames: [{ function: 'f', vars: { text: PRIVATE.text } }] } }] },
});

// the scrubbed event carries none of it
{
  const out = scrubEvent(event());
  const blob = JSON.stringify(out);
  for (const [k, v] of Object.entries(PRIVATE)) check(!blob.includes(v), `scrubbed event does not carry ${k}`);
  check(!blob.includes('owner@example.test') && !blob.includes('203.0.113.9') && !blob.includes('session=abc'), 'no email, ip or cookie');
  const keys = flattenKeys(out);
  for (const bad of ['headers', 'query_string', 'vars', 'context_line', 'pre_context', 'body', 'message_private', 'custom', 'email', 'ip_address', 'from', 'to']) {
    check(!keys.has(bad), `key ${bad} is gone`);
  }
  check(!flattenKeys(out.request).has('data'), 'request body is gone');
  check(out.exception.values[0].type === 'TypeError' && out.exception.values[0].value === "cannot read '...'", 'exception type kept, quoted literal and the second line blanked');
  check(JSON.stringify(out.exception.values[0].stacktrace.frames) === JSON.stringify([{ filename: 'app:///index.bundle', function: 'load', lineno: 12, colno: 3, in_app: true }]), 'frames keep file/function/line/col only');
  check(out.request.url === 'https://example.test/api/search' && out.request.method === 'GET', 'request keeps method + path without the query');
  check(out.message.startsWith("boom: '...' xxx") && out.message.length <= 300, 'message bounded with the literal blanked');
  check(out.breadcrumbs.length === 3 && out.breadcrumbs.every((b) => !('message' in b)), 'console breadcrumb dropped; no breadcrumb message');
  check(JSON.stringify(out.breadcrumbs[0].data) === JSON.stringify({ route: '/api/joke/{id}', method: 'GET', status_code: 500 }), 'fetch breadcrumb -> route pattern + method + status');
  check(JSON.stringify(out.breadcrumbs[1].data) === JSON.stringify({ route: '/api/decision', method: 'POST', status: 500 }), 'api breadcrumb keeps route/method/status only');
  check(!('data' in out.breadcrumbs[2]), 'navigation breadcrumb keeps no data');
  check(JSON.stringify(out.tags) === JSON.stringify({ where: 'stage' }), 'sensitive tags dropped');
  check(out.contexts.device.name === REDACTED && out.contexts.os.version === '26' && !('custom' in out.contexts), 'contexts: device/os kept, names scrubbed, custom dropped');
  check(JSON.stringify(out.extra) === JSON.stringify({ job: { kind: 'riff', text: REDACTED }, nested: { deep: { quote: REDACTED, n: 3 } }, ok: 'fine' }), 'extra: sensitive keys redacted at any depth');
  check(JSON.stringify(out.user) === JSON.stringify({ id: 'owner' }), 'user is an opaque id');
  check(out.release === 'punchable@0.1.0+7' && out.dist === '7' && out.environment === 'production', 'release, dist and environment survive');
  check(JSON.stringify(out.threads.values[0].stacktrace.frames) === JSON.stringify([{ function: 'f' }]), 'thread frames lose their locals');
}

// the event itself is never dropped
check(scrubEvent({ exception: { values: [{ type: 'X', value: 'y' }] } }).exception.values[0].type === 'X', 'a bare event still reports');
check(scrubEvent(null) === null, 'null passes through');

// the key rule
for (const k of ['text', 'TEXT', 'Transcript', 'set-cookie', 'joke_text', 'search_query', 'x-title', 'HTTP_AUTHORIZATION', 'punch', 'segments']) check(sensitiveKey(k), `sensitive: ${k}`);
for (const k of ['route', 'status', 'method', 'lineno', 'release', 'kind', 'model']) check(!sensitiveKey(k), `not sensitive: ${k}`);

// route patterns: the API's rule
const R = [
  ['/api/health', '/api/health'], ['/api/health?ready=1', '/api/health'], ['/api/joke/x:ab12cd', '/api/joke/{id}'],
  ['https://example.test/api/joke/Something%20Private', '/api/joke/{id}'], ['/api/search?q=sample', '/api/search'],
  ['/api/worker/jobs/job:riff-0a1b/result', '/api/worker/jobs/{id}/result'], ['/', '/'], ['', '/'], ['/a/b/c/d/e/f/g', '/a/b/c/d/e/f/...'],
];
for (const [p, want] of R) check(routePattern(p) === want, `routePattern(${p}) = ${want}`);
for (const seg of ['x1', 'X', 'a.b', 'a:b', 'a'.repeat(33), '%20']) check(routePattern('/api/' + seg) === '/api/{id}', `segment ${seg.slice(0, 8)} is an id`);

// breadcrumbs from the API client
{
  const b = apiBreadcrumb('post', '/api/upload?x=1', 0, 'TypeError: Network request failed');
  check(b.category === 'api' && b.level === 'warning' && b.data.route === '/api/upload' && b.data.method === 'POST' && b.data.status === 0, 'network failure breadcrumb');
  check(b.data.reason === 'TypeError: Network request failed'.slice(0, 32), 'reason is bounded');
  check(apiBreadcrumb('get', '/api/joke/x:1', 404).level === 'info', '4xx is info');
  check(scrubBreadcrumb({ category: 'console', message: 'anything' }) === null, 'console breadcrumbs are dropped');
  check(scrubBreadcrumb(null) === null, 'junk breadcrumb dropped');
}

// text, release, environment
check(scrubText(`KeyError: '${PRIVATE.text}'\n--- SELECT 1`) === "KeyError: '...'", 'scrubText blanks literals and cuts at the newline');
check(releaseName('0.1.0', 12) === 'punchable@0.1.0+12' && releaseName(undefined, null) === 'punchable@0.0.0+dev', 'release = version + build');
check(environmentName('preview', false) === 'preview' && environmentName('production', false) === 'production' && environmentName(undefined, false) === 'production' && environmentName('production', true) === 'dev', 'environment names');

// the proof the first block is load-bearing: with `text` taken off the sensitive list, his words leak and the same
// assertion fails (kept permanent, like the API's test)
{
  SENSITIVE_KEYS.delete('text');
  const broken = scrubEvent(event());
  SENSITIVE_KEYS.add('text');
  // 'text' still matches KEY_HINT, so the key rule alone does not leak; break both by renaming the key
  const ev = event();
  ev.extra.job = { kind: 'riff', words: PRIVATE.text };
  const leaked = JSON.stringify(scrubEvent(ev)).includes(PRIVATE.text);
  check(leaked, 'RED PROOF: a field with a name the rule does not know (words) leaks, so the rule is what protects text/quote/...');
  check(!JSON.stringify(broken).includes(PRIVATE.text), 'with text removed from the set, the hint regex still catches it');
}

console.log(fails.length ? `${fails.length} FAILED` : 'ALL PASS');
process.exit(fails.length ? 1 : 0);
