import { $ } from "bun";
import { bunEnv, bunExe, tempDir } from "harness";

test("child_process ipc", async () => {
  const output = await $`${bunExe()} ${import.meta.dir}/fixtures/ipc_fixture.js`.text();
  // node (v23.4.0) has identical output
  expect(output).toMatchInlineSnapshot(`
    "Parent received: {"status":"Child process started"}
    Child process exited with code 0
    send returned false
    uncaughtException ERR_IPC_CHANNEL_CLOSED
    cb ERR_IPC_CHANNEL_CLOSED
    "
  `);
});

describe.concurrent("subprocess.channel", () => {
  const files = parent => ({
    "child.cjs": `process.on("message", () => {}); process.send("ready");`,
    "parent.cjs": `
      const child = require("child_process").fork(__dirname + "/child.cjs");
      process.on("exit", () => { console.log("exit"); child.kill(); });
      child.unref();
      ${parent}
    `,
  });
  async function run(parent) {
    using dir = tempDir("ipc-channel-ref", files(parent));
    await using proc = Bun.spawn({ cmd: [bunExe(), "parent.cjs"], env: bunEnv, cwd: String(dir), stderr: "pipe" });
    const [stdout, stderr, exitCode] = await Promise.all([proc.stdout.text(), proc.stderr.text(), proc.exited]);
    return { stdout, stderr, exitCode };
  }

  test("an open channel keeps the parent alive after subprocess.unref()", async () => {
    const result = await run(`child.on("message", m => { console.log(m); child.disconnect(); });`);
    expect(result).toEqual({ stdout: "ready\nexit\n", stderr: "", exitCode: 0 });
  });

  test("channel.unref() lets the parent exit while the child runs", async () => {
    const result = await run(`child.channel.unref();`);
    expect(result).toEqual({ stdout: "exit\n", stderr: "", exitCode: 0 });
  });

  test("channel.ref() undoes channel.unref()", async () => {
    const result = await run(`child.channel.unref(); child.channel.ref(); child.on("message", m => { console.log(m); child.disconnect(); });`);
    expect(result).toEqual({ stdout: "ready\nexit\n", stderr: "", exitCode: 0 });
  });
});
