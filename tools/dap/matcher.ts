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

    const excludes = expected['$array.excludes'];
    if (excludes !== undefined) {
      if (!Array.isArray(excludes)) return fail('array.excludes requires array value');
      if (!Array.isArray(actual)) return fail(`expected array, got ${tn(actual)}`);

      for (let fi = 0; fi < excludes.length; fi++) {
        const forbidden = excludes[fi];
        for (let i = 0; i < actual.length; i++) {
          const result = match(forbidden, actual[i], `${at}[${i}]`);
          if (result.success) {
            return fail(`array.excludes[${fi}] matched actual[${i}]: ${JSON.stringify(forbidden)}`);
          }
        }
      }
      return succeed();
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

/** `number` → `0x…` string; `0x` + hex digits → number. No other string forms. */
export function hex(value: unknown): Json {
  if (typeof value === 'number') {
    if (!Number.isFinite(value) || Number.isNaN(value))
      throw new Error('hex: number must be finite');
    const n = Math.trunc(value);
    if (n < 0) throw new Error('hex: negative integers are not supported');
    return '0x' + n.toString(16);
  }
  if (typeof value === 'string') {
    const s = value.trim();
    if (!/^0x[0-9a-f]+$/i.test(s))
      throw new Error(`hex: expected 0x followed by hex digits, got ${JSON.stringify(value)}`);
    return parseInt(s, 16);
  }
  throw new Error(`hex: expected number or 0x hex string, got ${typeof value}`);
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
