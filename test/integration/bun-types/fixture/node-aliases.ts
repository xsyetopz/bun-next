import assert from "bun:assert";
import * as bunCrypto from "bun:crypto";
import { EventEmitter } from "bun:events";
import fs, { readFileSync, type Stats } from "bun:fs";
import { readFile } from "bun:fs/promises";
import type { IncomingMessage } from "bun:http";
import path from "bun:path";
import { setTimeout as sleep } from "bun:timers/promises";
import * as nodeCrypto from "node:crypto";
import * as nodeFs from "node:fs";
import * as bunFs from "bun:fs";
import { expectType } from "./utilities";

assert.equal(path.join("a", "b"), "a/b");
class Emitter extends EventEmitter {}
expectType(new Emitter().on("x", () => {})).is<Emitter>();
expectType(fs.statSync(".")).is<Stats>();
expectType(readFileSync("x", "utf8")).is<string>();
expectType(readFile("x")).is<Promise<NonSharedBuffer>>();
expectType(sleep(1, 1)).is<Promise<number>>();
expectType<IncomingMessage["headers"]>().is<import("node:http").IncomingHttpHeaders>();
expectType(bunFs).is<typeof nodeFs>();
expectType(bunCrypto).is<typeof nodeCrypto>();
