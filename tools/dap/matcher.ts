export type Json = null | boolean | number | string | JsonArray | JsonObject;
export type JsonArray = Json[];
export type JsonObject = { [k: string]: Json };
export type CaptureMap = Record<string, Json>;

const HANDLEBARS_RE = /^\{\{([\s\S]+)\}\}$/;
const CAPTURE_RE = /^\$\{\{([\s\S]+)\}\}$/;
const IDENTIFIER_RE = /^[a-zA-Z_]\w*$/;

export function substitutePlaceholders(input: Json, captures: CaptureMap): Json {
  if (typeof input === 'string') {
    const expression = parseHandlebarsExpression(input);
    if (expression !== null) return executeSnippet(expression, captures);
  }

  if (Array.isArray(input)) return input.map((v) => substitutePlaceholders(v, captures));
  if (input && typeof input === 'object') {
    const out: Record<string, Json> = {};
    for (const [k, v] of Object.entries(input))
      out[k] = substitutePlaceholders(v as Json, captures);
    return out;
  }
  return input;
}

export type MatchResult =
  | {
      success: true;
      captures: CaptureMap;
    }
  | {
      success: false;
      at: string;
      reason: string;
    };

export function match(expected: Json, actual: Json, at = ''): MatchResult {
  const succeed = (captures: CaptureMap = {}): MatchResult => ({ success: true, captures });
  const fail = (reason: string): MatchResult => ({ success: false, at, reason });

  const captureName = getCaptureName(expected);
  if (captureName !== null) return succeed({ [captureName]: actual });

  if (Array.isArray(expected)) {
    if (!Array.isArray(actual)) return fail(`expected array, got ${tn(actual)}`);
    if (actual.length !== expected.length)
      return fail(`expected array of size ${expected.length}, got ${actual.length}`);
    return allOf(expected.map((e, i) => match(e, actual[i], `${at}[${i}]`)));
  }

  function tryCompareSpecial(expected: JsonObject, actual: Json): MatchResult | null {
    const contains = expected['$array.contains'];
    if (contains) {
      if (!Array.isArray(contains)) return fail('array.contains requires array value');
      if (!Array.isArray(actual)) return fail(`expected array, got ${tn(actual)}`);

      return allOf(
        contains.map((e, ei) => {
          for (let i = 0; i < actual.length; i++) {
            const result = match(e, actual[i], `${at}[${i}]`);
            if (result.success) return result;
          }
          return fail(`item ${ei} not found: ${JSON.stringify(e)}`);
        })
      );
    }

    return null;
  }

  if (typeof expected === 'object' && expected !== null) {
    const special = tryCompareSpecial(expected, actual);
    if (special) return special;

    if (typeof actual !== 'object' || actual === null)
      return fail(`expected object, got ${tn(actual)}`);
    return allOf(
      Object.keys(expected).map((key) => match(expected[key], actual[key], `${at}.${key}`))
    );
  }

  if (!Object.is(expected, actual))
    return fail(`expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  return succeed();
}

function getCaptureName(value: Json): string | null {
  if (typeof value !== 'string') return null;
  const match = value.match(CAPTURE_RE);
  if (!match) return null;
  const expression = match[1].trim();
  return IDENTIFIER_RE.test(expression) ? expression : null;
}

function tn(value: Json) {
  if (Array.isArray(value)) return 'array';
  if (value === null) return 'null';
  return typeof value;
}

function allOf(matches: MatchResult[]) {
  const success: MatchResult = { success: true, captures: {} };
  for (const match of matches) {
    if (!match.success) return match;
    Object.assign(success.captures, match.captures);
  }
  return success;
}

function parseHandlebarsExpression(value: string): string | null {
  const match = value.match(HANDLEBARS_RE);
  if (!match) return null;
  return match[1].trim();
}

/** Bidirectional hex: hex strings (0x…) ↔ number; numbers ↔ 0x strings; decimal strings ↔ 0x strings. */
export function hex(value: unknown): Json {
  if (typeof value === 'number') {
    if (!Number.isFinite(value) || Number.isNaN(value))
      throw new Error('hex: number must be finite');
    const neg = value < 0;
    const abs = Math.floor(Math.abs(value));
    const h = abs.toString(16);
    return (neg ? '-0x' : '0x') + h;
  }
  if (typeof value === 'string') {
    const s = value.trim();
    if (/^0x[0-9a-fA-F]+$/.test(s)) {
      const n = parseInt(s.slice(2), 16);
      if (!Number.isFinite(n)) throw new Error(`hex: invalid literal ${JSON.stringify(value)}`);
      return n;
    }
    if (/^-0x[0-9a-fA-F]+$/.test(s)) {
      const n = parseInt(s.slice(1), 16);
      if (!Number.isFinite(n)) throw new Error(`hex: invalid literal ${JSON.stringify(value)}`);
      return n;
    }
    if (/^-?[0-9]+$/.test(s)) {
      const n = parseInt(s, 10);
      if (!Number.isFinite(n)) throw new Error(`hex: invalid literal ${JSON.stringify(value)}`);
      return hex(n);
    }
    throw new Error(`hex: unrecognised string ${JSON.stringify(value)}`);
  }
  throw new Error(`hex: expected number or string, got ${typeof value}`);
}

export function executeSnippet(code: string, captures: CaptureMap): Json {
  const t = code.trim();
  const body = /^\s*return\b/.test(t) || /;/.test(t) || /\n/.test(t) ? t : `return (${t});`;
  try {
    const keys = Object.keys(captures);
    const fn = new Function(...keys, 'hex', body);
    const result = fn(...Object.values(captures), hex);
    if (result === undefined) throw new Error('returned undefined');
    return result as Json;
  } catch (e) {
    const m = e instanceof Error ? e.message : String(e);
    throw new Error(`evaluation error: ${m}\n\n${code}`);
  }
}
