// stealth-v2.js — automation-detection mitigations injected before any
// page script runs. Pinned by SHA in browser_session.rs::stealth_script_sha256.
// When Chrome CDP changes break this, update the script AND the test
// expecting the new SHA in the same commit.

(() => {
  // 1) navigator.webdriver = false. The headless flag is the canonical
  //    detection vector; many sites refuse to load when this is true.
  Object.defineProperty(navigator, 'webdriver', {
    get: () => false,
    configurable: true,
  });

  // 2) Chrome runtime — replace the empty stub with a plausible object.
  if (!window.chrome || !window.chrome.runtime) {
    window.chrome = window.chrome || {};
    window.chrome.runtime = { id: undefined };
  }

  // 3) navigator.permissions.query — headless Chromium reports
  //    'denied' for notifications when the prompt is supposed to fire.
  const originalQuery = navigator.permissions
    ? navigator.permissions.query.bind(navigator.permissions)
    : null;
  if (originalQuery) {
    // Guard `Notification` — it's undefined inside cross-origin iframes
    // with restrictive feature policy. Calling `Notification.permission`
    // would throw a louder ReferenceError than the original 'denied'.
    navigator.permissions.query = (params) => {
      if (params && params.name === 'notifications') {
        const state = (typeof Notification !== 'undefined') ? Notification.permission : 'denied';
        return Promise.resolve({ state });
      }
      return originalQuery(params);
    };
  }

  // 4) Plugins length — 0 is a tell. Synthesize a minimal plugins array.
  Object.defineProperty(navigator, 'plugins', {
    get: () => [
      { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer' },
      { name: 'Chrome PDF Viewer', filename: 'internal-pdf-viewer' },
      { name: 'Native Client', filename: 'internal-nacl-plugin' },
    ],
    configurable: true,
  });

  // 5) Languages — headless reports an empty list on some builds.
  Object.defineProperty(navigator, 'languages', {
    get: () => ['en-US', 'en'],
    configurable: true,
  });

  // 6) WebGL vendor/renderer — headless reports 'Google SwiftShader'.
  //    Override to a plausible Intel UHD signature.
  const getParameter = WebGLRenderingContext.prototype.getParameter;
  WebGLRenderingContext.prototype.getParameter = function (parameter) {
    // UNMASKED_VENDOR_WEBGL = 37445; UNMASKED_RENDERER_WEBGL = 37446
    if (parameter === 37445) return 'Intel Inc.';
    if (parameter === 37446) return 'Intel Iris OpenGL Engine';
    return getParameter.call(this, parameter);
  };
  if (typeof WebGL2RenderingContext !== 'undefined') {
    const getParameter2 = WebGL2RenderingContext.prototype.getParameter;
    WebGL2RenderingContext.prototype.getParameter = function (parameter) {
      if (parameter === 37445) return 'Intel Inc.';
      if (parameter === 37446) return 'Intel Iris OpenGL Engine';
      return getParameter2.call(this, parameter);
    };
  }
})();
