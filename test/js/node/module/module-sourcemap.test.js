const { describe, test, expect } = require("bun:test");
const { bunEnv, bunExe, expectRssDeltaBelow } = require("harness");

test("SourceMap is available from node:module", () => {
  const module = require("node:module");
  expect(module.SourceMap).toBeDefined();
  expect(typeof module.SourceMap).toBe("function");
});

test("SourceMap from require('module') works", () => {
  const module = require("module");
  expect(module.SourceMap).toBeDefined();
  expect(typeof module.SourceMap).toBe("function");
});

test("Can create SourceMap instance from node:module", () => {
  const { SourceMap } = require("node:module");
  const payload = {
    version: 3,
    sources: ["test.js"],
    names: [],
    mappings: "AAAA",
  };

  const sourceMap = new SourceMap(payload);
  expect(sourceMap).toBeInstanceOf(SourceMap);
  expect(sourceMap.payload).toBe(payload);
});

test.concurrent("new SourceMap(payload) does not leak sources/names", async () => {
  const code = /* js */ `
    const { SourceMap } = require("module");
    const base = Buffer.alloc(256 * 1024, "a").toString();
    function once(i) { new SourceMap({ version: 3, sources: [base + i], names: [base + (i + 1)], mappings: "AAAA" }); }
    for (let i = 0; i < 20; i++) once(i);
    Bun.gc(true);
    const before = process.memoryUsage.rss();
    for (let i = 0; i < 300; i++) once(i);
    Bun.gc(true);
    console.log(JSON.stringify({ deltaMiB: (process.memoryUsage.rss() - before) / 1024 / 1024 }));
  `;

  // Unfixed: ~158 MiB. Fixed: allocator slack only.
  await expectRssDeltaBelow(["--smol", "-e", code], { release: 60, debug: 80 });
});

describe("source maps support", () => {
  const script = `
    const module = require("node:module");
    const report = () => console.log(JSON.stringify([process.sourceMapsEnabled, module.getSourceMapsSupport()]));
    report();
    module.setSourceMapsSupport(true, { nodeModules: true });
    report();
    process.setSourceMapsEnabled(false);
    report();
    const support = module.getSourceMapsSupport();
    console.log(Object.isFrozen(support), Object.getPrototypeOf(support), support === module.getSourceMapsSupport());
  `;

  test.concurrent.each([
    [[], false],
    [["--enable-source-maps"], true],
    [["--enable-source-maps", "--no-enable-source-maps"], false],
  ])("reports the setting under %j and follows the setters", async (flags, initial) => {
    await using proc = Bun.spawn({ cmd: [bunExe(), ...flags, "-e", script], env: bunEnv, stderr: "pipe" });
    const [stdout, stderr, exitCode] = await Promise.all([proc.stdout.text(), proc.stderr.text(), proc.exited]);
    const all = value => ({ enabled: value, nodeModules: value, generatedCode: value });
    expect({ stdout, stderr }).toEqual({
      stdout: [
        JSON.stringify([initial, all(initial)]),
        JSON.stringify([true, { enabled: true, nodeModules: true, generatedCode: false }]),
        JSON.stringify([false, all(false)]),
        "true null true",
        "",
      ].join("\n"),
      stderr: "",
    });
    expect(exitCode).toBe(0);
  });

  test("process.sourceMapsEnabled is an enumerable getter", () => {
    const descriptor = Object.getOwnPropertyDescriptor(process, "sourceMapsEnabled");
    expect(descriptor).toMatchObject({ enumerable: true, configurable: true, set: undefined });
    expect(typeof descriptor.get).toBe("function");
  });

  test("setSourceMapsSupport validates its arguments", () => {
    const { setSourceMapsSupport } = require("node:module");
    expect(setSourceMapsSupport.length).toBe(1);
    for (const [args, name] of [
      [[1], "enabled"],
      [[true, null], "options"],
      [[true, []], "options"],
      [[true, { nodeModules: 1 }], "options.nodeModules"],
      [[true, { generatedCode: "yes" }], "options.generatedCode"],
    ]) {
      expect(() => setSourceMapsSupport(...args)).toThrow(
        expect.objectContaining({ code: "ERR_INVALID_ARG_TYPE", message: expect.stringContaining(`"${name}"`) }),
      );
    }
  });
});
