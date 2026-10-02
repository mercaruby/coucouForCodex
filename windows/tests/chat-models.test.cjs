// Public catalog controller with deferred synthetic bridges. No DOM or network.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const test = require("node:test");
const ts = require("typescript");
const compiled = ts.transpileModule(fs.readFileSync(path.join(__dirname, "../src/core/chat-models.ts"), "utf8"), {
  compilerOptions: { module:ts.ModuleKind.CommonJS, target:ts.ScriptTarget.ES2022 },
}).outputText;
const sample = [ {slug:"gpt-6-astra",displayName:"Astra"}, {slug:"gpt-5.6-luna",displayName:"Luna"} ];
const session = (generation=1,connected=true,sharing=true) => ({ generation,connected,sharing,email:null,subject:null,clientId:null });
function deferred() {
  let resolve, reject;
  const promise = new Promise((res,rej) => { resolve=res; reject=rej; });
  return { promise,resolve,reject };
}
const flush = () => new Promise(resolve => setImmediate(resolve));
function fixture(backend="chatgpt") {
  const State = { settings:{chatBackend:backend} };
  const calls = { session:0,models:0,updates:0,changes:0 };
  const Bridge = {
    chatgptSession:async () => { calls.session++; return session(); },
    chatgptModels:async () => { calls.models++; return sample; },
  };
  const Usage = { sessionGeneration:1, session:session(), updateSession(value) {
    calls.updates++; this.sessionGeneration=value.generation;
    this.session={connected:value.connected,sharing:value.sharing,generation:value.generation};
  } };
  const deps = { "./bridge":{Bridge}, "./state":{State}, "./usage":{Usage} };
  const exportsFixture = {};
  vm.runInNewContext(compiled, { exports:exportsFixture,require:name=>deps[name] });
  const catalog = new exportsFixture.ChatModelCatalog(() => { calls.changes++; });
  return { catalog,State,Bridge,Usage,calls };
}

test("invisible views and explicit API backend never request ChatGPT metadata", async () => {
  const hidden = fixture(); hidden.catalog.ensure(false); await flush();
  assert.equal(hidden.calls.session,0);
  const api = fixture("api"); api.catalog.ensure(true); await flush();
  assert.equal(api.calls.session,0); assert.equal(api.calls.models,0);
  assert.equal(api.State.settings.chatBackend,"api");
});

test("disconnected or identity-only sessions never load a plan catalog", async () => {
  for (const [connected,sharing] of [[false,false],[true,false]]) {
    const fx = fixture(); fx.Bridge.chatgptSession=async()=>session(1,connected,sharing);
    fx.catalog.ensure(true); await flush();
    assert.equal(fx.calls.models,0); assert.equal(fx.catalog.available,false);
    assert.equal(fx.catalog.loading,false); assert.equal(fx.catalog.models.length,0);
  }
});

test("eligible catalog defaults to exact Luna and does not invent it when absent", async () => {
  const fx = fixture(); fx.catalog.ensure(true); await flush();
  assert.equal(fx.catalog.available,true); assert.equal(fx.catalog.automatic.slug,"gpt-5.6-luna");
  const absent = fixture(); absent.Bridge.chatgptModels=async()=>[{slug:"account-only-model",displayName:"Available"}];
  absent.catalog.ensure(true); await flush();
  assert.equal(absent.catalog.automatic.slug,"account-only-model");
  assert.equal(absent.catalog.models.length,1);
});

test("cached catalog does not repeatedly fetch and concurrent retries cannot duplicate a load", async () => {
  const fx = fixture(); fx.catalog.ensure(true); await flush();
  fx.catalog.ensure(true); fx.catalog.ensure(true); await flush();
  assert.equal(fx.calls.session,1); assert.equal(fx.calls.models,1);
  const hold=deferred(); fx.Bridge.chatgptModels=async()=>{fx.calls.models++; return hold.promise;};
  fx.catalog.retry(); await flush(); fx.catalog.retry(); fx.catalog.ensure(true);
  assert.equal(fx.calls.session,2); assert.equal(fx.calls.models,2);
  assert.equal(fx.catalog.available,false);
  hold.resolve(sample); await flush(); assert.equal(fx.catalog.available,true);
});

test("a stale model result cannot resurrect a catalog after generation changes", async () => {
  const fx=fixture(), hold=deferred(); fx.Bridge.chatgptModels=()=>hold.promise;
  fx.catalog.ensure(true); await flush();
  fx.Usage.sessionGeneration=2; fx.catalog.observe();
  hold.resolve(sample); await flush();
  assert.equal(fx.catalog.models.length,0); assert.equal(fx.catalog.available,false);
  fx.Bridge.chatgptSession=async()=>session(2);
  fx.Bridge.chatgptModels=async()=>[{slug:"generation-2-model",displayName:"New"}];
  fx.catalog.ensure(true); await flush();
  assert.equal(fx.catalog.automatic.slug,"generation-2-model");
});

test("stale session metadata cannot overwrite a newer generation", async () => {
  const fx=fixture(), hold=deferred(); fx.Bridge.chatgptSession=()=>hold.promise;
  fx.catalog.ensure(true);
  fx.Usage.sessionGeneration=2; fx.catalog.observe(); hold.resolve(session(1)); await flush();
  assert.equal(fx.Usage.sessionGeneration,2); assert.equal(fx.calls.updates,0);
  assert.equal(fx.calls.models,0); assert.equal(fx.catalog.available,false);
});

test("an external switch to API invalidates a pending ChatGPT session without changing backend", async () => {
  const fx=fixture(), hold=deferred(); fx.Bridge.chatgptSession=()=>hold.promise;
  fx.catalog.ensure(true);
  fx.State.settings.chatBackend="api"; fx.catalog.observe(); hold.resolve(session()); await flush();
  assert.equal(fx.State.settings.chatBackend,"api"); assert.equal(fx.calls.updates,0);
  assert.equal(fx.calls.models,0); assert.equal(fx.catalog.available,false);
});

test("login pending cancels old reads and blocks catalog until the login operation ends", async () => {
  const fx=fixture(), hold=deferred(); fx.Bridge.chatgptModels=()=>hold.promise;
  fx.catalog.ensure(true); await flush(); fx.catalog.setLoginBusy(true);
  hold.resolve(sample); await flush(); fx.catalog.ensure(true); await flush();
  assert.equal(fx.catalog.models.length,0); assert.equal(fx.catalog.available,false);
  assert.equal(fx.calls.session,1);
  fx.Bridge.chatgptModels=async()=>sample;
  fx.catalog.setLoginBusy(false); fx.catalog.ensure(true); await flush();
  assert.equal(fx.catalog.available,true); assert.equal(fx.calls.session,2);
});

test("catalog failure is redacted and a deliberate retry can recover", async () => {
  const fx=fixture(); fx.Bridge.chatgptModels=async()=>{throw new Error("synthetic-secret-marker");};
  fx.catalog.ensure(true); await flush();
  assert.equal(fx.catalog.available,false); assert.equal(fx.catalog.loading,false);
  assert.ok(fx.catalog.error); assert.ok(!fx.catalog.error.includes("synthetic-secret-marker"));
  fx.Bridge.chatgptModels=async()=>sample;
  fx.catalog.retry(); await flush(); assert.equal(fx.catalog.available,true); assert.equal(fx.catalog.error,null);
});

test("empty catalogs remain unavailable rather than presenting a fabricated default model", async () => {
  const fx=fixture(); fx.Bridge.chatgptModels=async()=>[];
  fx.catalog.ensure(true); await flush();
  assert.equal(fx.catalog.available,false); assert.equal(fx.catalog.automatic,undefined);
  assert.ok(fx.catalog.error); assert.equal(fx.catalog.loading,false);
});

test("a permission change at the same generation still invalidates pending model reads", async () => {
  const fx=fixture(), hold=deferred(); fx.Bridge.chatgptModels=()=>hold.promise;
  fx.catalog.ensure(true); await flush();
  fx.Usage.session={connected:true,sharing:false,generation:1}; fx.catalog.observe();
  hold.resolve(sample); await flush();
  assert.equal(fx.catalog.available,false); assert.equal(fx.catalog.models.length,0);
  assert.equal(fx.Usage.sessionGeneration,1); assert.equal(fx.Usage.session.sharing,false);
});
