// node's Web Locks API (worker_threads.locks, navigator.locks). One lock manager serves every thread of the process.
const { validateAbortSignal, validateFunction } = require("internal/validators");
const dc = require("node:diagnostics_channel");

const nativeRequest = $newCppFunction("NodeLocks.cpp", "jsFunctionLocksRequest", 5);
const nativeRelease = $newCppFunction("NodeLocks.cpp", "jsFunctionLocksRelease", 1);
const nativeAbort = $newCppFunction("NodeLocks.cpp", "jsFunctionLocksAbort", 1);
const nativeQuery = $newCppFunction("NodeLocks.cpp", "jsFunctionLocksQuery", 0);

const startChannel = dc.channel("locks.request.start");
const grantChannel = dc.channel("locks.request.grant");
const missChannel = dc.channel("locks.request.miss");
const endChannel = dc.channel("locks.request.end");

const kConstruct = Symbol("kConstruct");
const kName = Symbol("kName");
const kMode = Symbol("kMode");

type Waiter = { granted: PromiseWithResolvers<void>; released: PromiseWithResolvers<never> };
const waiters = new Map<number, Waiter>();

function noop() {}

function codedTypeError(message: string, code: string) {
  const error = new TypeError(message);
  (error as any).code = code;
  return error;
}

function convertLockOptions(options: any) {
  if (options != null && typeof options !== "object" && typeof options !== "function") {
    throw codedTypeError("Value cannot be converted to a dictionary", "ERR_INVALID_ARG_TYPE");
  }
  const ifAvailable = !!options?.ifAvailable;
  let mode = options?.mode;
  if (mode === undefined) {
    mode = "exclusive";
  } else {
    if (typeof mode === "symbol") {
      throw codedTypeError("mode is a Symbol and cannot be converted to a string.", "ERR_INVALID_ARG_TYPE");
    }
    mode = `${mode}`;
    if (mode !== "exclusive" && mode !== "shared") {
      throw codedTypeError(`mode '${mode}' is not a valid enum value of type LockMode.`, "ERR_INVALID_ARG_VALUE");
    }
  }
  const signal = options?.signal;
  if (signal !== undefined && (signal === null || (typeof signal !== "object" && typeof signal !== "function"))) {
    throw codedTypeError("signal is not an object.", "ERR_INVALID_ARG_TYPE");
  }
  const steal = !!options?.steal;
  return { mode, ifAvailable, steal, signal };
}

// https://w3c.github.io/web-locks/#api-lock
class Lock {
  constructor(symbol = undefined, name?: string, mode?: string) {
    if (symbol !== kConstruct) throw $ERR_ILLEGAL_CONSTRUCTOR();
    this[kName] = name;
    this[kMode] = mode;
  }

  get name() {
    if (this instanceof Lock) return this[kName];
    throw $ERR_INVALID_THIS("Lock");
  }

  get mode() {
    if (this instanceof Lock) return this[kMode];
    throw $ERR_INVALID_THIS("Lock");
  }
}

// Called from the lock manager on this thread's event loop. `granted` false means a steal took the held lock.
function settle(id: number, granted: boolean) {
  const waiter = waiters.get(id);
  if (waiter === undefined) {
    if (granted) nativeRelease(id);
    return;
  }
  if (granted) waiter.granted.resolve();
  else waiter.released.reject(new DOMException("The operation was aborted", "AbortError"));
}

async function waitForGrant(id: number, waiter: Waiter, signal: AbortSignal | undefined) {
  let onAbort;
  if (signal) {
    onAbort = () => {
      if (!nativeAbort(id)) return;
      waiters.delete(id);
      waiter.granted.reject(signal.reason);
    };
    signal.addEventListener("abort", onAbort, { once: true });
  }
  try {
    await waiter.granted.promise;
  } finally {
    signal?.removeEventListener("abort", onAbort);
  }
  if (signal?.aborted) {
    waiters.delete(id);
    nativeRelease(id);
    throw signal.reason;
  }
}

async function acquire(name, mode, ifAvailable, steal, signal, callback) {
  const { threadId } = require("node:worker_threads");
  const clientId = `node-${process.pid}-${threadId}`;
  let id = nativeRequest(name, clientId, mode === "exclusive", steal, ifAvailable);
  if (id === 0) {
    if (missChannel.hasSubscribers) missChannel.publish({ name, mode });
    return await callback(null);
  }

  const waiter: Waiter = { granted: Promise.withResolvers(), released: Promise.withResolvers() };
  waiter.released.promise.then(undefined, noop);
  const grantedNow = id < 0;
  if (grantedNow) id = -id;
  waiters.set(id, waiter);
  if (!grantedNow) await waitForGrant(id, waiter, signal);

  if (grantChannel.hasSubscribers) grantChannel.publish({ name, mode });
  const done = (async () => {
    try {
      return await callback(new Lock(kConstruct, name, mode));
    } finally {
      waiters.delete(id);
      nativeRelease(id);
    }
  })();
  return await Promise.race([done, waiter.released.promise]);
}

// https://w3c.github.io/web-locks/#api-lock-manager
class LockManager {
  constructor(symbol = undefined) {
    if (symbol !== kConstruct) throw $ERR_ILLEGAL_CONSTRUCTOR();
  }

  // https://w3c.github.io/web-locks/#api-lock-manager-request
  async request(name, options, callback = undefined) {
    if (callback === undefined) {
      callback = options;
      options = undefined;
    }
    if (typeof name === "symbol") {
      throw codedTypeError("Value is a Symbol and cannot be converted to a string.", "ERR_INVALID_ARG_TYPE");
    }
    name = `${name}`;
    validateFunction(callback, "callback");
    if (typeof options === "function") options = undefined;
    const { mode, ifAvailable, steal, signal } = convertLockOptions(options);
    validateAbortSignal(signal, "options.signal");
    signal?.throwIfAborted();

    if (name[0] === "-") {
      throw new DOMException("Lock name may not start with hyphen", "NotSupportedError");
    }
    if (ifAvailable && steal) {
      throw new DOMException("ifAvailable and steal are mutually exclusive", "NotSupportedError");
    }
    if (mode !== "exclusive" && steal) {
      throw new DOMException('mode: "shared" and steal are mutually exclusive', "NotSupportedError");
    }
    if (signal && (steal || ifAvailable)) {
      throw new DOMException("signal cannot be used with steal or ifAvailable", "NotSupportedError");
    }

    if (startChannel.hasSubscribers) startChannel.publish({ name, mode });
    let result;
    try {
      result = await acquire(name, mode, ifAvailable, steal, signal, callback);
    } catch (error) {
      if (endChannel.hasSubscribers) endChannel.publish({ name, mode, ifAvailable, steal, error });
      throw error;
    }
    if (endChannel.hasSubscribers) endChannel.publish({ name, mode, ifAvailable, steal, error: undefined });
    return result;
  }

  // https://w3c.github.io/web-locks/#api-lock-manager-query
  async query() {
    if (this instanceof LockManager) return nativeQuery();
    throw $ERR_INVALID_THIS("LockManager");
  }
}

const enumerable = { __proto__: null, enumerable: true };
const toStringTag = (value: string) => ({
  __proto__: null,
  value,
  writable: false,
  enumerable: false,
  configurable: true,
});
Object.defineProperties(Lock.prototype, {
  name: enumerable,
  mode: enumerable,
  [Symbol.toStringTag]: toStringTag("Lock"),
});
Object.defineProperties(LockManager.prototype, {
  request: enumerable,
  query: enumerable,
  [Symbol.toStringTag]: toStringTag("LockManager"),
});

export default {
  Lock,
  LockManager,
  locks: new LockManager(kConstruct),
  settle,
};
