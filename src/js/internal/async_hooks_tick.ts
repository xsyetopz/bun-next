// Bridge between node:async_hooks createHook() and the resources that report
// `init`: process.nextTick (builtins/ProcessObjectInternals.ts) reads
// `tickInitHooks` directly, so its hot path pays only an array-length check
// when no hook is enabled. setTimeout, setInterval and setImmediate call
// `emitTimerInit` from native code, which is registered only while a hook is.
//
// The array identity must stay stable (push/splice only, never reassign):
// the nextTick closure captures it once at setup.
//
// TickObject, Timeout and Immediate `init` events are delivered; promise and
// native resource events are still unimplemented.
type InitHook = (asyncId: number, type: string, triggerAsyncId: number, resource: object) => void;

const tickInitHooks: InitHook[] = [];
let nextAsyncId = 1;

const setTimerInitHook: (hook: ((resource: object, type: string) => void) | undefined) => void = $newCppFunction(
  "node/NodeTimers.cpp",
  "jsFunctionSetTimerInitHook",
  1,
);

// Node treats a throwing init hook as fatal.
function emitTimerInit(resource: object, type: string) {
  const asyncId = ++nextAsyncId;
  const hooks = tickInitHooks.slice();
  for (let i = 0; i < hooks.length; i++) {
    try {
      hooks[i](asyncId, type, 0, resource);
    } catch (err: any) {
      try {
        console.error(typeof err?.stack === "string" ? err.stack : err);
      } catch {}
      process.exit(1);
    }
  }
}

export default {
  tickInitHooks,
  newAsyncId() {
    return ++nextAsyncId;
  },
  addInitHook(hook: InitHook) {
    tickInitHooks.push(hook);
    if (tickInitHooks.length === 1) setTimerInitHook(emitTimerInit);
  },
  removeInitHook(hook: InitHook) {
    const index = tickInitHooks.indexOf(hook);
    if (index !== -1) tickInitHooks.splice(index, 1);
    if (tickInitHooks.length === 0) setTimerInitHook(undefined);
  },
};
