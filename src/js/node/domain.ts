const ObjectDefineProperty = Object.defineProperty;

type AnyFunction = (...args: any[]) => unknown;

declare global {
  namespace $Node {
    interface Domain extends EventEmitter {
      members: EventEmitter[];
      add(emitter: EventEmitter): void;
      remove(emitter: EventEmitter): void;
      bind(fn: AnyFunction): () => void;
      intercept(fn: AnyFunction): (err?: unknown) => void;
      run(fn: AnyFunction, ...args: unknown[]): unknown;
      dispose(): this;
      enter(): this;
      exit(): this;
    }
  }
}

interface DomainError extends Error {
  domainEmitter?: unknown;
  domainThrown?: boolean;
}

interface DomainModule {
  Domain: typeof Domain;
  createDomain(): $Node.Domain;
  create(): $Node.Domain;
  _stack: $Node.Domain[];
  active: $Node.Domain | null;
}

const EventEmitter = require("node:events");
const kEmitError = Symbol("kEmitError");

type DomainMember = $Node.EventEmitter & { domain?: $Node.Domain | null };

class Domain extends EventEmitter {
  members: DomainMember[] = [];
  declare [kEmitError]: (this: unknown, e?: unknown) => void;

  constructor() {
    super();
    const d = this;
    // The listener added to members: `this` is the emitter that failed.
    ObjectDefineProperty(this, kEmitError, {
      __proto__: null,
      value: function emitError(this: unknown, e?: unknown) {
        e ||= $ERR_UNHANDLED_ERROR();
        if (typeof e === "object") {
          (e as DomainError).domainEmitter = this;
          ObjectDefineProperty(e, "domain", {
            __proto__: null,
            configurable: true,
            enumerable: false,
            value: d,
            writable: true,
          });
          (e as DomainError).domainThrown = false;
        }
        d.emit("error", e);
      },
    });
  }

  add(emitter: DomainMember) {
    if (emitter.domain === this) return;
    // An emitter belongs to one domain at a time, as in Node.
    emitter.domain?.remove(emitter);
    ObjectDefineProperty(emitter, "domain", {
      __proto__: null,
      configurable: true,
      enumerable: false,
      value: this,
      writable: true,
    });
    this.members.push(emitter);
    emitter.on("error", this[kEmitError]);
  }

  remove(emitter: DomainMember) {
    emitter.domain = null;
    const index = this.members.indexOf(emitter);
    if (index !== -1) this.members.splice(index, 1);
    emitter.removeListener("error", this[kEmitError]);
  }

  bind(fn: AnyFunction) {
    const emitError = this[kEmitError];
    return function () {
      var args = Array.prototype.slice.$call(arguments);
      try {
        fn.$apply(null, args);
      } catch (err) {
        emitError(err);
      }
    };
  }

  intercept(fn: AnyFunction) {
    const emitError = this[kEmitError];
    return function (err?: unknown) {
      if (err) {
        emitError(err);
      } else {
        var args = Array.prototype.slice.$call(arguments, 1);
        try {
          fn.$apply(null, args);
        } catch (err) {
          emitError(err);
        }
      }
    };
  }

  run(fn: AnyFunction, ...args: unknown[]) {
    this.enter();
    try {
      return fn.$apply(this, args);
    } catch (err) {
      this[kEmitError](err);
    } finally {
      this.exit();
    }
  }

  dispose() {
    this.removeAllListeners();
    return this;
  }

  enter() {
    stack.push(this);
    domain.active = process.domain = this;
    return this;
  }

  exit() {
    const index = stack.lastIndexOf(this);
    if (index === -1) return this;
    stack.splice(index, stack.length);
    domain.active = process.domain = stack.length ? stack[stack.length - 1] : null;
    return this;
  }
}

var domain = {} as DomainModule;
domain.Domain = Domain;
domain.createDomain = domain.create = function () {
  return new Domain();
};

// Domains entered via enter()/run() and not yet exited, innermost last.
// process.domain mirrors the top of the stack like in node so other modules
// can observe the currently active domain.
const stack: $Node.Domain[] = [];
domain._stack = stack;
domain.active = null;

export default domain;
