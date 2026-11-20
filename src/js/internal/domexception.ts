// https://github.com/nodejs/node/blob/v26.13.1/lib/internal/per_context/domexception.js#L210

const NumberIsFinite = Number.isFinite;
const NumberIsNaN = Number.isNaN;
const ObjectDefineProperties = Object.defineProperties;

function toDouble(options: object, key: "quota" | "requested"): number | null {
  if (!(key in options)) return null;
  const value = +options[key];
  if (!NumberIsFinite(value)) {
    throw new TypeError(
      `Cannot convert options.${key} to a double: the value is ${NumberIsNaN(value) ? "NaN" : "Infinity"}`,
    );
  }
  if (value < 0) throw new RangeError(`options.${key} must not be negative`);
  return value;
}

class QuotaExceededError extends DOMException {
  #quota: number | null;
  #requested: number | null;

  constructor(message = "", options: { quota?: number; requested?: number } | null | undefined = { __proto__: null }) {
    super(message, "QuotaExceededError");
    let quota: number | null = null;
    let requested: number | null = null;
    if (options !== null && options !== undefined) {
      quota = toDouble(options, "quota");
      requested = toDouble(options, "requested");
    }
    if (quota !== null && requested !== null && requested < quota) {
      throw new RangeError("options.requested must not be less than options.quota");
    }
    this.#quota = quota;
    this.#requested = requested;
  }

  get quota() {
    if (!(#quota in this)) throw $ERR_INVALID_THIS("QuotaExceededError");
    return this.#quota;
  }

  get requested() {
    if (!(#requested in this)) throw $ERR_INVALID_THIS("QuotaExceededError");
    return this.#requested;
  }
}

ObjectDefineProperties(QuotaExceededError.prototype, {
  [Symbol.toStringTag]: { __proto__: null, configurable: true, value: "QuotaExceededError" },
  quota: { __proto__: null, enumerable: true, configurable: true },
  requested: { __proto__: null, enumerable: true, configurable: true },
});

export default { QuotaExceededError };
