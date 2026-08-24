/* Load the vendored Mermaid runtime only on pages that contain diagrams. */
(function () {
  let runtimePromise;
  let configured = false;
  const ISSUE_URL = /^https:\/\/github\.com\/encero-systems\/incan\/issues\/\d+$/;

  function collectIssueLinks(source) {
    const links = new Map();
    const code = source.querySelector("code")?.textContent || source.textContent;
    const directive = /^\s*click\s+([A-Za-z0-9_-]+)\s+href\s+"([^"]+)"/gm;
    let match = directive.exec(code);
    while (match) {
      const [, nodeId, url] = match;
      if (ISSUE_URL.test(url)) links.set(nodeId, url);
      match = directive.exec(code);
    }
    return links;
  }

  function bindIssueLink(link, url, nodeId, label) {
    link.setAttribute("aria-label", `Open ${label || nodeId} on GitHub`);
    link.removeAttribute("target");
    link.removeAttribute("rel");
    if (link.dataset.incanIssueLinkBound === "true") return;
    link.dataset.incanIssueLinkBound = "true";
    link.addEventListener("click", (event) => {
      if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      event.preventDefault();
      window.location.assign(url);
    });
  }

  function linkIssueNodes(host, issueLinks) {
    issueLinks.forEach((url, nodeId) => {
      host.querySelectorAll(".node[id]").forEach((node) => {
        if (!node.id.includes(`-flowchart-${nodeId}-`)) return;
        const existingLink = node.closest("a");
        if (existingLink) {
          const existingUrl = existingLink.getAttribute("href")
            || existingLink.getAttributeNS("http://www.w3.org/1999/xlink", "href");
          if (existingUrl === url) {
            bindIssueLink(existingLink, url, nodeId, node.textContent?.trim());
          }
          return;
        }
        const link = document.createElementNS("http://www.w3.org/2000/svg", "a");
        link.setAttribute("href", url);
        bindIssueLink(link, url, nodeId, node.textContent?.trim());
        node.parentNode?.replaceChild(link, node);
        link.appendChild(node);
      });
    });
  }

  function createZoomButton(label, accessibleLabel) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "inc-diagram-control";
    button.textContent = label;
    button.setAttribute("aria-label", accessibleLabel);
    return button;
  }

  function enableDiagramZoom(host) {
    const svg = host.querySelector?.("svg");
    if (!svg || host.dataset.incanDiagramZoomBound === "true") return;
    host.dataset.incanDiagramZoomBound = "true";

    const controls = document.createElement("div");
    controls.className = "inc-diagram-controls";
    controls.setAttribute("role", "group");
    controls.setAttribute("aria-label", "Diagram zoom");

    const decrease = createZoomButton("−", "Zoom out");
    const reset = createZoomButton("Fit", "Reset diagram zoom to fit");
    const increase = createZoomButton("+", "Zoom in");
    const status = document.createElement("output");
    status.className = "inc-diagram-zoom-status";
    status.setAttribute("aria-live", "polite");
    controls.append(decrease, reset, increase, status);

    const viewport = document.createElement("div");
    viewport.className = "inc-diagram-viewport";
    const stage = document.createElement("div");
    stage.className = "inc-diagram-stage";
    host.insertBefore(controls, svg);
    host.insertBefore(viewport, svg);
    stage.appendChild(svg);
    viewport.appendChild(stage);

    const zoomLevels = [0.75, 1, 1.25, 1.5, 2, 3, 4];
    let zoomIndex = zoomLevels.indexOf(1);

    function applyZoom() {
      const zoom = zoomLevels[zoomIndex];
      stage.style.width = `${zoom * 100}%`;
      status.value = `${Math.round(zoom * 100)}%`;
      status.textContent = status.value;
      decrease.disabled = zoomIndex === 0;
      increase.disabled = zoomIndex === zoomLevels.length - 1;
      reset.disabled = zoomIndex === zoomLevels.indexOf(1);
    }

    decrease.addEventListener("click", () => {
      zoomIndex = Math.max(0, zoomIndex - 1);
      applyZoom();
    });
    increase.addEventListener("click", () => {
      zoomIndex = Math.min(zoomLevels.length - 1, zoomIndex + 1);
      applyZoom();
    });
    reset.addEventListener("click", () => {
      zoomIndex = zoomLevels.indexOf(1);
      applyZoom();
    });
    applyZoom();
  }

  function loadRuntime() {
    if (window.mermaid) return Promise.resolve(window.mermaid);
    if (runtimePromise) return runtimePromise;

    runtimePromise = new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = "/shared/vendor/mermaid.min.js";
      script.addEventListener("load", () => {
        const runtime = window.mermaid || globalThis.mermaid;
        if (runtime) {
          resolve(runtime);
        } else {
          script.remove();
          reject(new Error("The local diagram runtime loaded without exposing Mermaid"));
        }
      }, { once: true });
      script.addEventListener("error", () => {
        script.remove();
        reject(new Error("Could not load the local diagram runtime"));
      }, { once: true });
      document.head.appendChild(script);
    }).catch((error) => {
      runtimePromise = undefined;
      throw error;
    });
    return runtimePromise;
  }

  function canRenderSource(source) {
    return !source.closest?.("details:not([open])");
  }

  async function init() {
    const sources = Array.from(document.querySelectorAll("pre.inc-diagram:not([data-incan-mermaid-queued])"))
      .filter(canRenderSource);
    if (sources.length === 0) return;
    sources.forEach((source) => source.setAttribute?.("data-incan-mermaid-queued", "true"));

    try {
      const mermaid = await loadRuntime();
      const diagrams = Array.from(sources, (source) => {
        const host = document.createElement("div");
        host.className = "inc-diagram";
        host.textContent = source.querySelector("code")?.textContent || source.textContent;
        source.replaceWith(host);
        return { host, issueLinks: collectIssueLinks(source) };
      });
      if (!configured) {
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: "strict",
          theme: "base",
          themeVariables: {
            background: "#06070a",
            primaryColor: "#150f0a",
            primaryTextColor: "#e4ebf2",
            primaryBorderColor: "#ffc15a",
            secondaryColor: "#08171a",
            secondaryTextColor: "#e4ebf2",
            secondaryBorderColor: "#48f0ef",
            tertiaryColor: "#1a0910",
            tertiaryTextColor: "#e4ebf2",
            tertiaryBorderColor: "#ff5c69",
            lineColor: "#98a5b3",
            edgeLabelBackground: "#08090c",
            fontFamily: "Inter, system-ui, sans-serif",
          },
        });
        configured = true;
      }
      for (const { host, issueLinks } of diagrams) {
        try {
          await mermaid.run({ nodes: [host], suppressErrors: false });
          linkIssueNodes(host, issueLinks);
          enableDiagramZoom(host);
        } catch (error) {
          console.warn("Incan diagram rendering failed", error);
        }
      }
    } catch (error) {
      sources.forEach((source) => source.removeAttribute?.("data-incan-mermaid-queued"));
      console.warn("Incan diagram rendering failed", error);
    }
  }

  document.addEventListener?.("toggle", (event) => {
    const details = event.target;
    if (!(details instanceof HTMLDetailsElement) || !details.open) return;
    init();
  }, true);

  if (typeof window.document$ !== "undefined" && typeof window.document$.subscribe === "function") {
    window.document$.subscribe(init);
  } else if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
