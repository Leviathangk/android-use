import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const formulaPath = join(dirname(fileURLToPath(import.meta.url)), "..", "homebrew", "android-use.rb");

test("formula declares the AndroidUse class", async () => {
  assert.match(await readFile(formulaPath, "utf8"), /^class AndroidUse < Formula$/m);
});

test("formula has a sha256 placeholder for each platform asset", async () => {
  const text = await readFile(formulaPath, "utf8");
  const placeholders = text.match(/sha256 "REPLACE_WITH_SHA256_FROM_SHA256SUMS"/g) ?? [];
  assert.equal(placeholders.length, 2);
  assert.match(text, /macos-aarch64\.tar\.gz/);
  assert.match(text, /linux-x86_64\.tar\.gz/);
});
