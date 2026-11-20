// https://github.com/nodejs/node/blob/v26.13.1/src/node_webstorage.cc
// The schema and the statements are those of Node.js, so a file that either writes, the other reads.

const { Database } = require("../bun/sqlite");
const { QuotaExceededError } = require("internal/domexception");

const ObjectDefineProperties = Object.defineProperties;
const ObjectDefineProperty = Object.defineProperty;
const ReflectHas = Reflect.has;
const ReflectGet = Reflect.get;
const SafeMap = Map;
const SafeWeakMap = WeakMap;

const kConstructorKey = Symbol("kConstructorKey");
const kInMemoryPath = ":memory:";

const initSql = `
PRAGMA encoding = 'UTF-16le';
PRAGMA busy_timeout = 3000;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA temp_store = memory;
PRAGMA optimize;
CREATE TABLE IF NOT EXISTS nodejs_webstorage(
  key BLOB NOT NULL,
  value BLOB NOT NULL,
  PRIMARY KEY(key)
) STRICT;
CREATE TABLE IF NOT EXISTS nodejs_webstorage_state(
  max_size INTEGER NOT NULL DEFAULT 10485760,
  total_size INTEGER NOT NULL,
  schema_version INTEGER NOT NULL DEFAULT 0,
  single_row_ INTEGER NOT NULL DEFAULT 1 CHECK(single_row_ = 1),
  PRIMARY KEY(single_row_)
) STRICT;
CREATE TRIGGER IF NOT EXISTS nodejs_quota_insert AFTER INSERT ON nodejs_webstorage FOR EACH ROW BEGIN
  UPDATE nodejs_webstorage_state SET total_size = total_size + OCTET_LENGTH(NEW.key) + OCTET_LENGTH(NEW.value);
  SELECT RAISE(ABORT, 'QuotaExceeded') WHERE EXISTS (SELECT 1 FROM nodejs_webstorage_state WHERE total_size > max_size);
END;
CREATE TRIGGER IF NOT EXISTS nodejs_quota_update AFTER UPDATE ON nodejs_webstorage FOR EACH ROW BEGIN
  UPDATE nodejs_webstorage_state SET total_size = total_size +
    ((OCTET_LENGTH(NEW.key) + OCTET_LENGTH(NEW.value)) - (OCTET_LENGTH(OLD.key) + OCTET_LENGTH(OLD.value)));
  SELECT RAISE(ABORT, 'QuotaExceeded') WHERE EXISTS (SELECT 1 FROM nodejs_webstorage_state WHERE total_size > max_size);
END;
CREATE TRIGGER IF NOT EXISTS nodejs_quota_delete AFTER DELETE ON nodejs_webstorage FOR EACH ROW BEGIN
  UPDATE nodejs_webstorage_state SET total_size = total_size - (OCTET_LENGTH(OLD.key) + OCTET_LENGTH(OLD.value));
END;
INSERT OR IGNORE INTO nodejs_webstorage_state (total_size) VALUES (0);`;
const kCurrentSchemaVersion = 1;

type State = { location: string; db: any; symbols: Map<symbol, unknown> };

// Keyed by the proxy, which is the `this` of every method, and by its target, which the traps get.
const states = new SafeWeakMap<object, State>();

const encode = (text: string) => Buffer.from(text, "utf16le");
const decode = (bytes: Uint8Array) => Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength).toString("utf16le");

function stateOf(storage: unknown): State {
  const state = states.get(storage as object);
  if (state === undefined) throw new TypeError("Illegal invocation");
  return state;
}

function sqliteError(error: any) {
  return $ERR_INVALID_STATE(error?.message ?? String(error));
}

function open(state: State) {
  if (state.db !== null) return state.db;
  let db;
  try {
    db = new Database(state.location);
    db.run(initSql);
    const { schema_version } = db.query("SELECT schema_version FROM nodejs_webstorage_state").get();
    if (schema_version > kCurrentSchemaVersion) {
      db.close();
      throw $ERR_INVALID_STATE("localStorage was created with a newer version of Node.js");
    }
    if (schema_version < kCurrentSchemaVersion) {
      db.run(`UPDATE nodejs_webstorage_state SET schema_version = ${kCurrentSchemaVersion};`);
    }
  } catch (error: any) {
    if (error?.code === "ERR_INVALID_STATE") throw error;
    db?.close();
    throw sqliteError(error);
  }
  return (state.db = db);
}

function load(state: State, key: string | symbol): string | null | unknown {
  if (typeof key === "symbol") return state.symbols.get(key);
  const row = open(state).query("SELECT value FROM nodejs_webstorage WHERE key = ? LIMIT 1").get(encode(key));
  return row ? decode(row.value) : null;
}

function store(state: State, key: string | symbol, value: unknown) {
  if (typeof key === "symbol") return void state.symbols.set(key, value);
  const text = `${value}`;
  const db = open(state);
  try {
    db.query(
      "INSERT INTO nodejs_webstorage (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value WHERE EXCLUDED.key = key",
    ).run(encode(key), encode(text));
  } catch (error: any) {
    if (`${error?.code}`.startsWith("SQLITE_CONSTRAINT")) {
      throw new QuotaExceededError("Setting the value exceeded the quota");
    }
    throw sqliteError(error);
  }
}

function remove(state: State, key: string | symbol) {
  if (typeof key === "symbol") return void state.symbols.delete(key);
  open(state).query("DELETE FROM nodejs_webstorage WHERE key = ?").run(encode(key));
}

function keys(state: State): string[] {
  return open(state)
    .query("SELECT key FROM nodejs_webstorage")
    .values()
    .map(([key]) => decode(key));
}

// V8's interceptors in Node.js: a name that the prototype chain has is not an item.
const isItemName = (key: string | symbol) => !ReflectHas(Storage.prototype, key);

const handler: ProxyHandler<object> = {
  get(target, key, receiver) {
    if (!isItemName(key)) return ReflectGet(target, key, receiver);
    return load(stateOf(target), key) ?? undefined;
  },
  set(target, key, value) {
    store(stateOf(target), key, value);
    return true;
  },
  has(target, key) {
    return !isItemName(key) || (load(stateOf(target), key) ?? null) !== null;
  },
  deleteProperty(target, key) {
    remove(stateOf(target), key);
    return true;
  },
  ownKeys(target) {
    return keys(stateOf(target));
  },
  getOwnPropertyDescriptor(target, key) {
    if (!isItemName(key)) return undefined;
    const value = load(stateOf(target), key);
    if (value === null || value === undefined) return undefined;
    return { value, writable: true, enumerable: true, configurable: true };
  },
  defineProperty(target, key, descriptor) {
    if ("value" in descriptor) store(stateOf(target), key, descriptor.value);
    return true;
  },
};

class Storage {
  constructor(key?: unknown, location?: string) {
    if (key !== kConstructorKey) throw $ERR_ILLEGAL_CONSTRUCTOR();
    const state = { location: location!, db: null, symbols: new SafeMap() };
    const proxy = new Proxy(this, handler);
    states.set(this, state);
    states.set(proxy, state);
    return proxy;
  }

  get length() {
    return open(stateOf(this)).query("SELECT count(*) AS n FROM nodejs_webstorage").get().n;
  }

  clear() {
    open(stateOf(this)).run("DELETE FROM nodejs_webstorage");
  }

  getItem(key: string) {
    const state = stateOf(this);
    if (arguments.length < 1) throw missingArgs("getItem", 1);
    return load(state, `${key}`);
  }

  key(index: number) {
    const state = stateOf(this);
    if (arguments.length < 1) throw missingArgs("key", 1);
    index = index | 0;
    if (index < 0) return null;
    const row = open(state).query("SELECT key FROM nodejs_webstorage LIMIT 1 OFFSET ?").values(index)[0];
    return row ? decode(row[0]) : null;
  }

  removeItem(key: string) {
    const state = stateOf(this);
    if (arguments.length < 1) throw missingArgs("removeItem", 1);
    remove(state, `${key}`);
  }

  setItem(key: string, value: string) {
    const state = stateOf(this);
    if (arguments.length < 2) throw missingArgs("setItem", 2);
    store(state, `${key}`, value);
  }
}

function missingArgs(method: string, count: number) {
  const error = $ERR_MISSING_ARGS(method);
  error.message = `Failed to execute '${method}' on 'Storage': ${count} argument${count > 1 ? "s" : ""} required`;
  return error;
}

ObjectDefineProperties(Storage.prototype, {
  [Symbol.toStringTag]: { __proto__: null, configurable: true, value: "Storage" },
  // util.inspect() shows the target of a proxy, which holds no items. Node.js shows the items.
  [Symbol.for("nodejs.util.inspect.custom")]: {
    __proto__: null,
    configurable: true,
    value: function inspect(depth, options, inspect) {
      const state = stateOf(this);
      const items = {};
      for (const key of keys(state)) {
        ObjectDefineProperty(items, key, { __proto__: null, enumerable: true, value: load(state, key) });
      }
      return `Storage ${inspect(items, options)}`;
    },
  },
  length: { __proto__: null, enumerable: true, configurable: false },
  clear: { __proto__: null, enumerable: true },
  getItem: { __proto__: null, enumerable: true },
  key: { __proto__: null, enumerable: true },
  removeItem: { __proto__: null, enumerable: true },
  setItem: { __proto__: null, enumerable: true },
});

let lazySessionStorage;

export default {
  Storage,
  get sessionStorage() {
    return (lazySessionStorage ??= new Storage(kConstructorKey, kInMemoryPath));
  },
};
