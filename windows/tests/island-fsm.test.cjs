// Actual FSM with a deterministic clock. No DOM, native bridge or credentials.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const test = require("node:test");
const ts = require("typescript");
const source = fs.readFileSync(path.join(__dirname, "../src/island/fsm.ts"), "utf8");
const compiled = ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022,
} }).outputText;

function fixture(seconds = 15, outside = true) {
  let now = 0;
  let nextId = 1;
  const pending = new Map();
  const exportsFixture = {};
  const clock = {
    setTimeout(callback, delay) {
      assert.ok(Number.isFinite(delay) && delay >= 0, "Timer delay must be valid");
      const id = nextId++;
      pending.set(id, { callback, at: now + delay });
      return id;
    },
    clearTimeout(id) { pending.delete(id); },
    advance(ms) {
      const target = now + ms;
      let callbacks = 0;
      while (true) {
        const due = [...pending.entries()].filter(([, timer]) => timer.at <= target)
          .sort((a, b) => a[1].at - b[1].at || a[0] - b[0])[0];
        if (!due) break;
        assert.ok(++callbacks < 1000, "Zero-delay timer loop is a regression");
        const [id, timer] = due;
        pending.delete(id);
        now = timer.at;
        timer.callback();
      }
      now = target;
    },
    pendingCount() { return pending.size; },
  };
  vm.runInNewContext(compiled, { exports: exportsFixture, window: clock });
  const fsm = new exportsFixture.IslandStateMachine();
  const transitions = [];
  // This is the actual wireFsm host contract: expanded/compact transitions
  // reschedule their outside-cursor timer when the cursor stays outside.
  fsm.onTransition = (from, to) => {
    transitions.push([from, to]);
    if (outside && (to === "home" || to === "petit")) fsm.mouseLeft();
  };
  fsm.configureAutoClose(seconds, outside);
  return {
    fsm, clock, transitions,
    configure(value) { fsm.configureAutoClose(value, outside); },
    enter() { outside = false; fsm.mouseEntered(); },
    leave() { outside = true; fsm.mouseLeft(); },
  };
}

test("ordinary default closes expanded at 15s and compact after another 60s", () => {
  const { fsm, clock, transitions } = fixture();
  fsm.forceHome();
  clock.advance(14999);
  assert.equal(fsm.state, "home");
  clock.advance(1);
  assert.equal(fsm.state, "petit");
  clock.advance(59999);
  assert.equal(fsm.state, "petit");
  clock.advance(1);
  assert.equal(fsm.state, "hidden");
  assert.deepEqual(transitions.map(([, to]) => to), ["home", "petit", "hidden"]);
});

test("Never keeps expanded and compact visible without auto-close timers", () => {
  const { fsm, clock } = fixture(0);
  fsm.forceHome();
  assert.equal(clock.pendingCount(), 0);
  clock.advance(7 * 24 * 60 * 60 * 1000);
  assert.equal(fsm.state, "home");
  fsm.forcePetit();
  assert.equal(clock.pendingCount(), 0);
  clock.advance(7 * 24 * 60 * 60 * 1000);
  assert.equal(fsm.state, "petit");
});

test("enabling Never cancels an expanded timer that is about to fire", () => {
  const fx = fixture();
  fx.fsm.forceHome();
  fx.clock.advance(14999);
  fx.configure(0);
  assert.equal(fx.clock.pendingCount(), 0);
  fx.clock.advance(24 * 60 * 60 * 1000);
  assert.equal(fx.fsm.state, "home");
});

test("enabling Never cancels an existing compact hide timer", () => {
  const fx = fixture();
  fx.fsm.forcePetit();
  fx.clock.advance(59999);
  fx.configure(0);
  assert.equal(fx.clock.pendingCount(), 0);
  fx.clock.advance(24 * 60 * 60 * 1000);
  assert.equal(fx.fsm.state, "petit");
});

test("reenabling auto-close while outside rearms the current expanded and compact states", () => {
  const home = fixture(0);
  home.fsm.forceHome();
  home.clock.advance(60000);
  home.configure(5);
  home.clock.advance(4999);
  assert.equal(home.fsm.state, "home");
  home.clock.advance(1);
  assert.equal(home.fsm.state, "petit");
  home.clock.advance(60000);
  assert.equal(home.fsm.state, "hidden");

  const compact = fixture(0);
  compact.fsm.forcePetit();
  compact.configure(5);
  compact.clock.advance(59999);
  assert.equal(compact.fsm.state, "petit");
  compact.clock.advance(1);
  assert.equal(compact.fsm.state, "hidden");
});

test("reenabling while hovered waits for mouse leave instead of hiding under the cursor", () => {
  const fx = fixture(0, false);
  fx.fsm.forceHome();
  fx.configure(5);
  assert.equal(fx.clock.pendingCount(), 0);
  fx.clock.advance(60000);
  assert.equal(fx.fsm.state, "home");
  fx.leave();
  fx.clock.advance(5000);
  assert.equal(fx.fsm.state, "petit");
  fx.enter();
  fx.clock.advance(60000);
  assert.equal(fx.fsm.state, "petit");
});

test("Never preserves explicit collapse, pause/hide and later event reveal", () => {
  const { fsm, clock } = fixture(0);
  fsm.forceHome();
  fsm.forcePetit();
  assert.equal(fsm.state, "petit");
  fsm.forceHidden();
  clock.advance(60000);
  assert.equal(fsm.state, "hidden");
  fsm.reveal();
  clock.advance(24 * 60 * 60 * 1000);
  assert.equal(fsm.state, "petit");
  assert.equal(clock.pendingCount(), 0);
});

test("pinned alerts remain open when timed auto-close is enabled", () => {
  const fx = fixture();
  fx.fsm.pinned = true;
  fx.fsm.forceHome();
  fx.clock.advance(120000);
  assert.equal(fx.fsm.state, "home");
  fx.fsm.pinned = false;
  fx.configure(5);
  fx.clock.advance(5000);
  assert.equal(fx.fsm.state, "petit");
});
