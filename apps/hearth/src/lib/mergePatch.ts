// JSON merge patches (RFC 7386), as Rust applies them to the settings: objects merge key by
// key, arrays and other values replace, and `null` removes a key (Rust then fills in its
// default). Used to show a settings change at once, before Rust answers.

type Json = Record<string, unknown>;

function isObject(value: unknown): value is Json {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** `target` with `patch` applied. `null` at the top level is ignored (Rust's default arrives soon). */
export function mergePatch<T>(target: T, patch: unknown, top = true): T {
  if (!isObject(patch)) return patch as T;
  const out: Json = isObject(target) ? { ...target } : {};
  for (const [key, value] of Object.entries(patch)) {
    if (value === null) {
      if (!top) delete out[key];
    } else if (isObject(value) && isObject(out[key])) {
      out[key] = mergePatch(out[key], value, false);
    } else {
      out[key] = value;
    }
  }
  return out as T;
}
