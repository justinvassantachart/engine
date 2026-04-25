export type Json = null | boolean | number | string | Json[] | { [k: string]: Json };
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

export function assertMatch(expected: Json, actual: Json, captures: CaptureMap, at = '$'): void {
  const isObj = (v: Json): v is Record<string, Json> =>
    typeof v === 'object' && v !== null && !Array.isArray(v);
  const isPlaceholder = (v: Json): v is string => typeof v === 'string' && PLACEHOLDER_RE.test(v);
  const fmt = (v: unknown) => {
    try {
      return JSON.stringify(v, null, 2);
    } catch {
      return String(v);
    }
  };

  if (isPlaceholder(expected)) {
    captures[expected.slice(2, -2)] = actual;
    return;
  }

  if (Array.isArray(expected)) {
    if (!Array.isArray(actual)) throw new Error(`${at}: expected array, got ${typeof actual}`);
    if (actual.length !== expected.length) {
      throw new Error(`${at}: expected array length ${expected.length}, got ${actual.length}`);
    }
    expected.forEach((v, i) => assertMatch(v, actual[i] as Json, captures, `${at}[${i}]`));
    return;
  }

  if (isObj(expected)) {
    if (!isObj(actual)) throw new Error(`${at}: expected object, got ${typeof actual}`);

    for (const [key, expectedValue] of Object.entries(expected)) {
      if (key === '$array.contains') {
        if (!Array.isArray(actual)) throw new Error(`${at}: array expected`);
        if (!Array.isArray(expectedValue))
          throw new Error(`${at}: $array.contains value must be array`);

        expectedValue.forEach((template, i) => {
          let matched = false;
          for (const candidate of actual) {
            const local = { ...captures };
            try {
              assertMatch(template as Json, candidate as Json, local, `${at}[${i}]`);
              Object.assign(captures, local);
              matched = true;
              break;
            } catch {}
          }
          if (!matched) {
            throw new Error(`${at}: $array.contains: element ${i} not found`);
          }
        });
        continue;
      }

      if (!(key in actual)) throw new Error(`${at}.${key}: missing key in actual object`);
      assertMatch(expectedValue as Json, actual[key] as Json, captures, `${at}.${key}`);
    }
    return;
  }

  if (!Object.is(expected, actual)) {
    throw new Error(`${at}: expected ${fmt(expected)}, got ${fmt(actual)}`);
  }
}
