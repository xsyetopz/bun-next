// Port of lib/internal/util/throttle.js in Node.js.
const {
  validateAbortSignal,
  validateBoolean,
  validateFunction,
  validateInteger,
  validateObject,
  validateOneOf,
} = require("internal/validators");
const { addAbortListener } = require("internal/abort_listener");
const { kEmptyObject } = require("internal/shared");
const { clearTimeout, setTimeout } = require("node:timers");

const TIMEOUT_MAX = 2 ** 31 - 1;
const PromisePrototypeThen = $Promise.prototype.$then;

type Call = ReturnType<typeof Promise.withResolvers<unknown>> & { args: unknown[] };

function abortError(cause?: unknown) {
  return cause === undefined ? $makeAbortError() : $makeAbortError(undefined, { cause });
}

function throttle(
  fn: (...args: unknown[]) => unknown,
  limit: number,
  interval: number,
  options: Record<string, unknown> = kEmptyObject,
) {
  validateFunction(fn, "fn");
  validateInteger(limit, "limit", 1);
  validateInteger(interval, "interval", 0, TIMEOUT_MAX);
  validateObject(options, "options");

  const {
    concurrency = Infinity,
    maxPending = Infinity,
    overflow = "queue",
    signal,
    strict = false,
  } = options as {
    concurrency?: number;
    maxPending?: number;
    overflow?: "queue" | "drop";
    signal?: AbortSignal;
    strict?: boolean;
  };

  if (concurrency !== Infinity) validateInteger(concurrency, "options.concurrency", 1);
  if (maxPending !== Infinity) validateInteger(maxPending, "options.maxPending", 0);
  validateOneOf(overflow, "options.overflow", ["queue", "drop"]);
  validateAbortSignal(signal, "options.signal");
  validateBoolean(strict, "options.strict");

  if (signal?.aborted) throw $makeAbortError(undefined, { cause: signal.reason });

  let aborted: Error | undefined;
  let activeCount = 0;
  let pendingCalls: (Call | undefined)[] = [];
  let pendingIndex = 0;
  let refed = true;
  let strictIndex = 0;
  let strictTicks: number[] = [];
  let timeout: ReturnType<typeof setTimeout> | undefined;
  let windowCount = 0;
  let windowStart: number | undefined;

  function pendingCount() {
    return pendingCalls.length - pendingIndex;
  }

  // Calls leave the queue by index so a long queue drains without quadratic shifting.
  function dequeue() {
    const call = pendingCalls[pendingIndex]!;
    pendingCalls[pendingIndex++] = undefined;
    if (pendingIndex === pendingCalls.length) {
      pendingCalls = [];
      pendingIndex = 0;
    } else if (pendingIndex > 1024 && pendingIndex * 2 >= pendingCalls.length) {
      pendingCalls = pendingCalls.slice(pendingIndex);
      pendingIndex = 0;
    }
    return call;
  }

  function rejectPending(error: unknown) {
    const calls = pendingCalls;
    const start = pendingIndex;
    pendingCalls = [];
    pendingIndex = 0;
    for (let i = start; i < calls.length; i++) calls[i]!.reject(error);
  }

  function cancelWithError(error: unknown) {
    if (timeout !== undefined) {
      clearTimeout(timeout);
      timeout = undefined;
    }
    strictIndex = 0;
    strictTicks = [];
    windowCount = 0;
    windowStart = undefined;
    rejectPending(error);
  }

  function cancel(reason?: unknown) {
    cancelWithError(abortError(reason));
  }

  function abort() {
    aborted ??= abortError(signal!.reason);
    cancelWithError(aborted);
  }

  function ref() {
    refed = true;
    timeout?.ref();
    return throttled;
  }

  function unref() {
    refed = false;
    timeout?.unref();
    return throttled;
  }

  function hasImmediateCapacity() {
    if (signal?.aborted || pendingCount() !== 0 || activeCount >= concurrency) return false;
    const now = performance.now();
    if (strict) {
      let index = strictIndex;
      while (index < strictTicks.length && now - strictTicks[index] >= interval) index++;
      return strictTicks.length - index < limit;
    }
    return windowStart === undefined || now - windowStart >= interval || windowCount < limit;
  }

  function pruneStrictTicks(now: number) {
    while (strictIndex < strictTicks.length && now - strictTicks[strictIndex] >= interval) strictIndex++;
    if (strictIndex === strictTicks.length) {
      strictIndex = 0;
      strictTicks = [];
    } else if (strictIndex > 1024 && strictIndex * 2 >= strictTicks.length) {
      strictTicks = strictTicks.slice(strictIndex);
      strictIndex = 0;
    }
  }

  function hasCapacity(now: number) {
    if (strict) {
      pruneStrictTicks(now);
      return strictTicks.length - strictIndex < limit;
    }
    if (windowStart === undefined || now - windowStart >= interval) {
      windowCount = 0;
      windowStart = now;
    }
    return windowCount < limit;
  }

  function reserve(now: number) {
    if (strict) strictTicks.push(now);
    else windowCount++;
  }

  function onSettled() {
    activeCount--;
    drain();
  }

  function invoke(call: Call) {
    activeCount++;
    let result: Promise<unknown>;
    try {
      result = Promise.$resolve(fn.$apply(throttled, call.args));
    } catch (error) {
      result = Promise.$reject(error);
    }
    call.resolve(result);
    PromisePrototypeThen.$call(result, onSettled, onSettled);
  }

  function schedule() {
    if (timeout !== undefined || pendingCount() === 0 || activeCount >= concurrency) return;
    const now = performance.now();
    if (hasCapacity(now)) {
      drain();
      return;
    }
    const delay = strict ? strictTicks[strictIndex] + interval - now : windowStart! + interval - now;
    timeout = setTimeout(onTimeout, delay);
    if (!refed) timeout.unref();
  }

  function drain() {
    if (signal?.aborted) {
      abort();
      return;
    }
    while (pendingCount() !== 0 && activeCount < concurrency) {
      const now = performance.now();
      if (!hasCapacity(now)) break;
      const call = dequeue();
      reserve(now);
      invoke(call);
    }
    schedule();
  }

  function onTimeout() {
    timeout = undefined;
    drain();
  }

  function throttled(...callArgs: unknown[]) {
    if (signal?.aborted) abort();
    if (aborted !== undefined) return Promise.$reject(aborted);

    const now = performance.now();
    const canInvoke = pendingCount() === 0 && activeCount < concurrency && hasCapacity(now);
    if (!canInvoke && (overflow === "drop" || pendingCount() >= maxPending)) {
      const dropped = Promise.$reject($ERR_THROTTLED());
      $pokePromiseAsHandled(dropped);
      return dropped;
    }

    const call = Promise.withResolvers<unknown>() as Call;
    call.args = callArgs;
    if (canInvoke) {
      reserve(now);
      invoke(call);
    } else {
      pendingCalls.push(call);
      schedule();
    }
    return call.promise;
  }

  const method = (value: Function) => ({ configurable: true, enumerable: true, writable: true, value });

  Object.defineProperties(throttled, {
    cancel: method(cancel),
    hasImmediateCapacity: method(hasImmediateCapacity),
    ref: method(ref),
    unref: method(unref),
    pending: {
      enumerable: false,
      get() {
        return pendingCount() === 0 ? null : pendingCalls[pendingCalls.length - 1]!.promise;
      },
    },
    pendingCount: { enumerable: false, get: pendingCount },
    activeCount: {
      enumerable: false,
      get() {
        return activeCount;
      },
    },
    length: { configurable: true, value: fn.length },
    name: { configurable: true, value: fn.name },
  });

  if (signal !== undefined) addAbortListener(signal, abort);

  return throttled;
}

export default throttle;
