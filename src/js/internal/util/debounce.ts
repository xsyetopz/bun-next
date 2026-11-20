// Port of lib/internal/util/debounce.js in Node.js.
const {
  validateAbortSignal,
  validateBoolean,
  validateFunction,
  validateInteger,
  validateObject,
} = require("internal/validators");
const { addAbortListener } = require("internal/abort_listener");
const { kEmptyObject } = require("internal/shared");
const { clearTimeout, setTimeout } = require("node:timers");

const TIMEOUT_MAX = 2 ** 31 - 1;

type Call = ReturnType<typeof Promise.withResolvers<unknown>>;

function abortError(cause?: unknown, message?: string) {
  return cause === undefined ? $makeAbortError(message) : $makeAbortError(message, { cause });
}

function debounce(fn: (...args: unknown[]) => unknown, wait: number, options: Record<string, unknown> = kEmptyObject) {
  validateFunction(fn, "fn");
  validateInteger(wait, "wait", 0, TIMEOUT_MAX);
  validateObject(options, "options");

  const {
    leading = false,
    rejectOnCancel = false,
    signal,
  } = options as {
    leading?: boolean;
    rejectOnCancel?: boolean;
    signal?: AbortSignal;
  };

  validateBoolean(leading, "options.leading");
  validateBoolean(rejectOnCancel, "options.rejectOnCancel");
  validateAbortSignal(signal, "options.signal");

  if (signal?.aborted) throw $makeAbortError(undefined, { cause: signal.reason });

  let args: unknown[] | undefined;
  let aborted: Error | undefined;
  let pendingCalls: Call[] = [];
  let refed = true;
  let timeout: ReturnType<typeof setTimeout> | undefined;

  function rejectPending(error: unknown) {
    const calls = pendingCalls;
    pendingCalls = [];
    for (let i = 0; i < calls.length; i++) calls[i].reject(error);
  }

  function cancelWithError(error: unknown) {
    if (timeout === undefined) return;
    clearTimeout(timeout);
    timeout = undefined;
    args = undefined;
    rejectPending(error);
  }

  function cancel(reason?: unknown) {
    if (timeout === undefined) return;
    cancelWithError(abortError(reason));
  }

  function abort() {
    aborted ??= abortError(signal!.reason);
    cancelWithError(aborted);
  }

  function ref() {
    refed = true;
    timeout?.ref();
    return debounced;
  }

  function unref() {
    refed = false;
    timeout?.unref();
    return debounced;
  }

  function invoke(callArgs: unknown[] | undefined, calls: Call[]) {
    if (signal?.aborted) {
      abort();
      for (let i = 0; i < calls.length; i++) calls[i].reject(aborted);
      return;
    }

    let result;
    try {
      result = fn.$apply(debounced, callArgs);
    } catch (error) {
      for (let i = 0; i < calls.length; i++) calls[i].reject(error);
      return;
    }

    for (let i = 0; i < calls.length; i++) calls[i].resolve(result);
  }

  function invokePending() {
    const callArgs = args;
    args = undefined;
    const calls = pendingCalls;
    pendingCalls = [];
    if (calls.length !== 0) invoke(callArgs, calls);
  }

  function onTimeout() {
    timeout = undefined;
    invokePending();
  }

  function flush() {
    if (pendingCalls.length === 0) return;
    clearTimeout(timeout);
    timeout = undefined;
    invokePending();
  }

  function debounced(...callArgs: unknown[]) {
    if (signal?.aborted) abort();
    if (aborted !== undefined) return Promise.$reject(aborted);

    const invokeNow = leading && timeout === undefined;
    if (timeout !== undefined) {
      if (pendingCalls.length !== 0) {
        $pokePromiseAsHandled(pendingCalls[pendingCalls.length - 1].promise);
        if (rejectOnCancel) rejectPending(abortError(undefined, "The debounced call was superseded"));
      }
      timeout.refresh();
    } else {
      timeout = setTimeout(onTimeout, wait);
      if (!refed) timeout.unref();
    }

    if (signal?.aborted) {
      abort();
      return Promise.$reject(aborted);
    }

    const call = Promise.withResolvers<unknown>();
    if (invokeNow) {
      invoke(callArgs, [call]);
    } else {
      pendingCalls.push(call);
      args = callArgs;
    }
    return call.promise;
  }

  const method = (value: Function) => ({ configurable: true, enumerable: true, writable: true, value });

  Object.defineProperties(debounced, {
    cancel: method(cancel),
    flush: method(flush),
    ref: method(ref),
    unref: method(unref),
    pending: {
      enumerable: false,
      get() {
        return pendingCalls.length === 0 ? null : pendingCalls[pendingCalls.length - 1].promise;
      },
    },
    pendingCount: {
      enumerable: false,
      get() {
        return pendingCalls.length;
      },
    },
    length: { configurable: true, value: fn.length },
    name: { configurable: true, value: fn.name },
  });

  if (signal !== undefined) addAbortListener(signal, abort);

  return debounced;
}

export default debounce;
