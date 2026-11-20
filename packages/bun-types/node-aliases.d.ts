// Each Node.js builtin `node:<name>` is also importable as `bun:<name>`, and
// these declarations give it the same types. `bun:sqlite` and `bun:test` are
// Bun's own modules, so they are not aliases of `node:sqlite` and `node:test`.

declare module "bun:assert" {
  import m = require("node:assert");
  export = m;
}
declare module "bun:assert/strict" {
  import m = require("node:assert/strict");
  export = m;
}
declare module "bun:async_hooks" {
  import m = require("node:async_hooks");
  export = m;
}
declare module "bun:buffer" {
  import m = require("node:buffer");
  export = m;
}
declare module "bun:child_process" {
  import m = require("node:child_process");
  export = m;
}
declare module "bun:cluster" {
  import m = require("node:cluster");
  export = m;
}
declare module "bun:console" {
  import m = require("node:console");
  export = m;
}
declare module "bun:constants" {
  import m = require("node:constants");
  export = m;
}
declare module "bun:crypto" {
  import m = require("node:crypto");
  export = m;
}
declare module "bun:dgram" {
  import m = require("node:dgram");
  export = m;
}
declare module "bun:diagnostics_channel" {
  import m = require("node:diagnostics_channel");
  export = m;
}
declare module "bun:dns" {
  import m = require("node:dns");
  export = m;
}
declare module "bun:dns/promises" {
  import m = require("node:dns/promises");
  export = m;
}
declare module "bun:domain" {
  import m = require("node:domain");
  export = m;
}
declare module "bun:events" {
  import m = require("node:events");
  export = m;
}
declare module "bun:fs" {
  import m = require("node:fs");
  export = m;
}
declare module "bun:fs/promises" {
  import m = require("node:fs/promises");
  export = m;
}
declare module "bun:http" {
  import m = require("node:http");
  export = m;
}
declare module "bun:http2" {
  import m = require("node:http2");
  export = m;
}
declare module "bun:https" {
  import m = require("node:https");
  export = m;
}
declare module "bun:inspector" {
  import m = require("node:inspector");
  export = m;
}
declare module "bun:inspector/promises" {
  import m = require("node:inspector/promises");
  export = m;
}
declare module "bun:module" {
  import m = require("node:module");
  export = m;
}
declare module "bun:net" {
  import m = require("node:net");
  export = m;
}
declare module "bun:os" {
  import m = require("node:os");
  export = m;
}
declare module "bun:path" {
  import m = require("node:path");
  export = m;
}
declare module "bun:path/posix" {
  import m = require("node:path/posix");
  export = m;
}
declare module "bun:path/win32" {
  import m = require("node:path/win32");
  export = m;
}
declare module "bun:perf_hooks" {
  import m = require("node:perf_hooks");
  export = m;
}
declare module "bun:process" {
  import m = require("node:process");
  export = m;
}
declare module "bun:punycode" {
  import m = require("node:punycode");
  export = m;
}
declare module "bun:querystring" {
  import m = require("node:querystring");
  export = m;
}
declare module "bun:readline" {
  import m = require("node:readline");
  export = m;
}
declare module "bun:readline/promises" {
  import m = require("node:readline/promises");
  export = m;
}
declare module "bun:repl" {
  import m = require("node:repl");
  export = m;
}
declare module "bun:stream" {
  import m = require("node:stream");
  export = m;
}
declare module "bun:stream/consumers" {
  import m = require("node:stream/consumers");
  export = m;
}
declare module "bun:stream/promises" {
  import m = require("node:stream/promises");
  export = m;
}
declare module "bun:stream/web" {
  import m = require("node:stream/web");
  export = m;
}
declare module "bun:string_decoder" {
  import m = require("node:string_decoder");
  export = m;
}
declare module "bun:sys" {
  import m = require("node:util");
  export = m;
}
declare module "bun:timers" {
  import m = require("node:timers");
  export = m;
}
declare module "bun:timers/promises" {
  import m = require("node:timers/promises");
  export = m;
}
declare module "bun:tls" {
  import m = require("node:tls");
  export = m;
}
declare module "bun:trace_events" {
  import m = require("node:trace_events");
  export = m;
}
declare module "bun:tty" {
  import m = require("node:tty");
  export = m;
}
declare module "bun:url" {
  import m = require("node:url");
  export = m;
}
declare module "bun:util" {
  import m = require("node:util");
  export = m;
}
declare module "bun:util/types" {
  import m = require("node:util/types");
  export = m;
}
declare module "bun:v8" {
  import m = require("node:v8");
  export = m;
}
declare module "bun:vm" {
  import m = require("node:vm");
  export = m;
}
declare module "bun:wasi" {
  import m = require("node:wasi");
  export = m;
}
declare module "bun:worker_threads" {
  import m = require("node:worker_threads");
  export = m;
}
declare module "bun:zlib" {
  import m = require("node:zlib");
  export = m;
}
