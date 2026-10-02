// Pure frontend policy tests. No browser, native bridge, OAuth or network.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const ts = require("typescript");
const source = fs.readFileSync(path.join(__dirname, "../src/core/chat-notice.ts"), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
} }).outputText;
const exportsFixture = {};
vm.runInNewContext(compiled, { exports: exportsFixture, Error });
const { chatNotice, MANAGE_USAGE_URL } = exportsFixture;
const marker = "synthetic-secret-marker";
const cases = [
  ["OpenAI reported a ChatGPT sharing limit for this app or account.", "limit", "usage"],
  ["OpenAI reported a rate limit for this request.", "temporary", "usage"],
  ["OpenAI reported a usage or rate limit for this request.", "temporary", "usage"],
  ["ChatGPT usage could not be checked right now.", "temporary", "chat"],
  ["Your ChatGPT account or workspace is temporarily unavailable.", "temporary", "chat"],
  ["ChatGPT is temporarily unavailable.", "temporary", "chat"],
  ["ChatGPT rejected this request.", "model", "settings"],
  ["ChatGPT plan usage is unavailable for this account, workspace or policy.", "restriction", "settings"],
  ["A ChatGPT policy or permission restriction prevented this request.", "restriction", "settings"],
  ["ChatGPT does not support this request route.", "restriction", "chat"],
  ["This app is not enabled for ChatGPT plan usage.", "restriction", "chat"],
  ["ChatGPT rejected this app registration.", "restriction", "chat"],
  ["ChatGPT could not validate the selected account context.", "connection", "settings"],
  ["Your connection does not allow this ChatGPT plan request.", "connection", "settings"],
  ["This model or message uses a capability unsupported by ChatGPT plan usage.", "model", "settings"],
  ["ChatGPT did not complete this response. Try again or check your account's usage limits.", "other", "chat"],
];
for (const [message, kind, action] of cases) {
  const result = chatNotice(new Error(`${message} ${marker}`), "chatgpt");
  assert.equal(result.kind, kind, message);
  assert.equal(result.action, action, message);
  assert.ok(!JSON.stringify(result).includes(marker));
  assert.ok(!/reconect|vuelve a conectar/i.test(result.detail), "Restriction/limit is not proof of expired authentication");
}
const limit = chatNotice(cases[0][0], "chatgpt");
assert.match(limit.detail, /modelo/);
assert.match(limit.detail, /elige otro/);
assert.match(limit.detail, /mensaje está guardado/);
const unknown = chatNotice(`${marker} raw provider details`, "chatgpt");
assert.equal(unknown.kind, "other");
assert.ok(!JSON.stringify(unknown).includes(marker));
assert.equal(chatNotice(cases[0][0], "api").kind, "other", "Subscription quota is not API billing state");
assert.equal(MANAGE_USAGE_URL, "https://chatgpt.com/settings/usage");
console.log(`PASS: ${cases.length} recovery mappings, redaction, model guidance and explicit backend separation`);
