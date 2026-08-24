import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";

const sourcePath = new URL("../docs/_snippets/javascripts/incan_mermaid.js", import.meta.url);
const runtimeSource = fs.readFileSync(sourcePath, "utf8");

function element(tagName) {
  const children = [];
  const listeners = new Map();
  return {
    tagName,
    children,
    className: "",
    dataset: {},
    style: {},
    textContent: "",
    append(...nodes) {
      nodes.forEach((node) => this.appendChild(node));
    },
    appendChild(node) {
      node.parentNode?.removeChild(node);
      children.push(node);
      node.parentNode = this;
      return node;
    },
    insertBefore(node, before) {
      node.parentNode?.removeChild(node);
      const index = children.indexOf(before);
      children.splice(index < 0 ? children.length : index, 0, node);
      node.parentNode = this;
      return node;
    },
    removeChild(node) {
      const index = children.indexOf(node);
      if (index >= 0) children.splice(index, 1);
      node.parentNode = undefined;
      return node;
    },
    querySelector(selector) {
      for (const child of children) {
        if (child.tagName === selector || child.className === selector.slice(1)) return child;
        const descendant = child.querySelector?.(selector);
        if (descendant) return descendant;
      }
      return null;
    },
    setAttribute(name, value) {
      this.attributes ??= {};
      this.attributes[name] = value;
    },
    getAttribute(name) {
      return this.attributes?.[name] ?? null;
    },
    addEventListener(type, listener) {
      listeners.set(type, listener);
    },
    dispatch(type) {
      listeners.get(type)?.();
    },
  };
}

function diagramSource(text) {
  return {
    textContent: text,
    querySelector(selector) {
      return selector === "code" ? { textContent: text } : null;
    },
    replaceWith(host) {
      this.replacement = host;
    },
  };
}

function deferredDiagramSource(text) {
  return {
    ...diagramSource(text),
    closest(selector) {
      return selector === "details:not([open])" ? {} : null;
    },
  };
}

const scripts = [];
const warnings = [];
let subscribed;
let sources = [diagramSource("flowchart LR; A-->B")];

const document = {
  head: {
    appendChild(script) {
      scripts.push(script);
    },
  },
  createElement(tagName) {
    if (tagName === "script") {
      const listeners = new Map();
      return {
        addEventListener(type, listener) {
          listeners.set(type, listener);
        },
        dispatch(type) {
          listeners.get(type)?.();
        },
        remove() {
          this.removed = true;
        },
      };
    }
    return element(tagName);
  },
  querySelectorAll() {
    return sources.filter((source) => !source.replacement);
  },
};

const window = {
  document$: {
    subscribe(handler) {
      subscribed = handler;
    },
  },
};

const context = vm.createContext({
  console: { warn: (...args) => warnings.push(args) },
  document,
  window,
});
vm.runInContext(runtimeSource, context, { filename: sourcePath.pathname });

assert.equal(typeof subscribed, "function", "Material navigation should register the diagram initializer");

const failedLoad = subscribed();
assert.equal(sources[0].replacement, undefined, "source must remain visible until the runtime loads");
assert.equal(scripts.length, 1, "the first navigation should request the vendored runtime once");
scripts[0].dispatch("error");
await failedLoad;
assert.equal(sources[0].replacement, undefined, "a failed runtime load must preserve the source diagram");
assert.equal(scripts[0].removed, true, "a failed script element should be removed before retry");

let initializeCalls = 0;
let runCalls = 0;
const mermaid = {
  initialize() {
    initializeCalls += 1;
  },
  async run({ nodes }) {
    runCalls += 1;
    assert.ok(nodes.length > 0, "Mermaid should receive the replacement diagram nodes");
    nodes.forEach((node) => node.appendChild(element("svg")));
  },
};

const retry = subscribed();
assert.equal(scripts.length, 2, "a later navigation should retry a transient runtime failure");
window.mermaid = mermaid;
context.mermaid = mermaid;
scripts[1].dispatch("load");
await retry;
assert.ok(sources[0].replacement, "a successful retry should replace the source with a rendered host");
assert.equal(initializeCalls, 1, "the shared runtime should be configured once");
assert.equal(runCalls, 1, "the recovered diagram should render once");

const firstHost = sources[0].replacement;
const zoomControls = firstHost.children.find((child) => child.className === "inc-diagram-controls");
const zoomStage = firstHost.querySelector(".inc-diagram-stage");
assert.ok(zoomControls, "a rendered diagram should expose accessible zoom controls");
assert.equal(zoomStage?.style.width, "100%", "a diagram should begin at its fitted size");
const [decrease, reset, increase, status] = zoomControls.children;
assert.equal(status.textContent, "100%", "the initial zoom label should state the fitted size");
assert.equal(reset.disabled, true, "reset should be unavailable while the diagram is fitted");
increase.dispatch("click");
assert.equal(zoomStage.style.width, "125%", "zoom in should widen the scrollable diagram stage");
assert.equal(status.textContent, "125%", "zoom in should update the accessible zoom label");
assert.equal(reset.disabled, false, "reset should become available after zooming");
reset.dispatch("click");
assert.equal(zoomStage.style.width, "100%", "reset should restore the fitted diagram size");
assert.equal(reset.disabled, true, "reset should become unavailable after restoring fit");
assert.equal(decrease.disabled, false, "zoom out should remain available from the fitted size");

sources = [diagramSource("sequenceDiagram; A->>B: next page")];
await subscribed();
assert.ok(sources[0].replacement, "instant navigation should render newly discovered diagrams");
assert.equal(initializeCalls, 1, "instant navigation should reuse the configured runtime");
assert.equal(runCalls, 2, "the next page should render through the existing runtime");

sources = [deferredDiagramSource("flowchart TD; pending-->visible")];
await subscribed();
assert.equal(sources[0].replacement, undefined, "a closed detail should defer its diagram until visible");
assert.equal(runCalls, 2, "a hidden diagram should not consume a Mermaid render pass");

sources[0].closest = () => null;
await subscribed();
assert.ok(sources[0].replacement, "an opened detail should render its deferred diagram");
assert.equal(runCalls, 3, "a revealed diagram should render through the existing runtime");
assert.equal(warnings.length, 1, "only the simulated transient failure should emit a warning");

console.log("Incan Mermaid runtime contract passed (failure recovery, zoom, and instant navigation)");
