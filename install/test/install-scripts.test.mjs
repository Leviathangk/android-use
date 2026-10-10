import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const installSh = join(dirname(fileURLToPath(import.meta.url)), "..", "install.sh");
const sh = spawnSync("sh", ["-c", ":"]);

test("install.sh has valid POSIX sh syntax", { skip: sh.error ? "sh is not available" : false }, () => {
  const r = spawnSync("sh", ["-n", installSh], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
});

test("install.sh verifies the checksum before extracting", async () => {
  const text = await readFile(installSh, "utf8");
  const verified = text.indexOf("SHA256 mismatch");
  assert.ok(verified > 0, "missing checksum mismatch check");
  assert.ok(verified < text.indexOf("tar -xzf"), "archive is extracted before it is verified");
});
