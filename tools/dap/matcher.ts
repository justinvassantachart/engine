export type Json = null | boolean | number | string | JsonArray | JsonObject;
export type JsonArray = Json[];
export type JsonObject = { [k: string]: Json };
export type CaptureMap = Record<string, Json>;

const PLACEHOLDER_RE = /^\{\{[a-zA-Z_]\w*\}\}$/;

export function substitutePlaceholders(input: Json, captures: CaptureMap): Json {
  if (typeof input === 'string' && PLACEHOLDER_RE.test(input)) {
    const name = input.slice(2, -2);
    if (!(name in captures)) throw new Error(`unbound placeholder ${input}`);
    return captures[name];
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

  if (isPlaceholder(expected)) return succeed({ [expected.slice(2, -2)]: actual });

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

function isPlaceholder(value: Json): value is string {
  return typeof value === 'string' && PLACEHOLDER_RE.test(value);
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
