const CUBISM_CORE_SCRIPT_PATH = "/live2dcubismcore.min.js";
const CUBISM_CORE_READY_TIMEOUT_MS = 5000;

declare global {
  interface Window {
    Live2DCubismCore?: unknown;
  }
}

let cubismCoreLoadPromise: Promise<void> | null = null;

function hasCubismCore(): boolean {
  return typeof window !== "undefined" && window.Live2DCubismCore !== undefined;
}

function findExistingCubismCoreScript(): HTMLScriptElement | null {
  const scripts = Array.from(document.scripts);

  for (const script of scripts) {
    const rawSrc = script.getAttribute("src");
    if (rawSrc === CUBISM_CORE_SCRIPT_PATH) {
      return script;
    }

    if (script.src.endsWith(CUBISM_CORE_SCRIPT_PATH)) {
      return script;
    }
  }

  return null;
}

function waitForCubismCore(timeoutMs = CUBISM_CORE_READY_TIMEOUT_MS): Promise<void> {
  if (hasCubismCore()) {
    return Promise.resolve();
  }

  return new Promise((resolve, reject) => {
    const deadline = Date.now() + timeoutMs;

    const checkReady = () => {
      if (hasCubismCore()) {
        resolve();
        return;
      }

      if (Date.now() >= deadline) {
        reject(new Error(`timed out waiting for ${CUBISM_CORE_SCRIPT_PATH} to initialize`));
        return;
      }

      window.setTimeout(checkReady, 25);
    };

    checkReady();
  });
}

function injectCubismCoreScript(): Promise<void> {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = CUBISM_CORE_SCRIPT_PATH;
    script.async = false;

    script.onload = () => {
      void waitForCubismCore().then(resolve).catch(reject);
    };
    script.onerror = () => {
      reject(new Error(`failed to load ${CUBISM_CORE_SCRIPT_PATH}`));
    };

    document.head.appendChild(script);
  });
}

export function ensureCubismCoreLoaded(): Promise<void> {
  if (typeof window === "undefined" || typeof document === "undefined") {
    return Promise.reject(new Error("Cubism Core can only be loaded in a browser context"));
  }

  if (hasCubismCore()) {
    return Promise.resolve();
  }

  if (cubismCoreLoadPromise) {
    return cubismCoreLoadPromise;
  }

  cubismCoreLoadPromise = (async () => {
    const existingScript = findExistingCubismCoreScript();

    if (existingScript) {
      await waitForCubismCore();
      return;
    }

    await injectCubismCoreScript();
  })().catch((error) => {
    cubismCoreLoadPromise = null;
    throw error;
  });

  return cubismCoreLoadPromise;
}
