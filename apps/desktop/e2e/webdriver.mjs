// Minimal WebDriver client for tauri-driver — deliberately without WebdriverIO
// (a deviation from the original design, documented in DEVELOPMENT.md): we only
// need a session, element lookup, click, text and script — Node built-ins are enough.
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

const PORT = 4444;
const BASE = `http://127.0.0.1:${PORT}/session`;

async function req(method, path, body) {
  const res = await fetch(`${BASE}${path}`, {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const json = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new Error(`WebDriver ${method} ${path}: ${res.status} ${JSON.stringify(json)}`);
  }
  return json.value;
}

/** Starts tauri-driver (which talks to msedgedriver itself) and a session. */
const driverUp = () =>
  fetch(`http://127.0.0.1:${PORT}/status`)
    .then((r) => r.ok)
    .catch(() => false);

export async function startSession({ application, driverBin, nativeDriver, env }) {
  // A driver from the previous test file may still be shutting down: wait
  // until the port is free, or the new driver cannot bind it.
  for (let i = 0; i < 50 && (await driverUp()); i++) await sleep(200);
  // `env` reaches the app: tauri-driver and msedgedriver pass it on.
  const driver = spawn(driverBin, ["--native-driver", nativeDriver], {
    stdio: "ignore",
    env: { ...process.env, ...env },
  });
  // Wait until the driver port answers.
  let up = false;
  for (let i = 0; i < 150 && !up; i++) {
    up = await driverUp();
    if (!up) await sleep(200);
  }
  if (!up) {
    driver.kill();
    throw new Error("tauri-driver does not respond on port 4444");
  }

  const session = await req("POST", "", {
    capabilities: {
      alwaysMatch: { "tauri:options": { application } },
    },
  });
  const id = session.sessionId;

  const s = {
    driver,
    id,
    async quit() {
      await req("DELETE", `/${id}`).catch(() => {});
      // Give msedgedriver time to end the app — otherwise a locked tg-app.exe
      // stays behind (next build: os error 5).
      await sleep(500);
      // Wait for the driver to be gone, so the next session can bind the port.
      const exited = new Promise((r) => driver.once("exit", r));
      driver.kill();
      await Promise.race([exited, sleep(5000)]);
    },
    /** Runs a script in the WebView; the `return` value comes back. */
    async exec(script, args = []) {
      return req("POST", `/${id}/execute/sync`, { script, args });
    },
    async find(css) {
      const el = await req("POST", `/${id}/element`, {
        using: "css selector",
        value: css,
      });
      return Object.values(el)[0];
    },
    async text(css) {
      const el = await s.find(css);
      return req("GET", `/${id}/element/${el}/text`);
    },
    /** Poll helper: waits until `fn` returns truthy (timeout 15 s). */
    async waitFor(fn, label = "condition") {
      for (let i = 0; i < 75; i++) {
        try {
          const v = await fn();
          if (v) return v;
        } catch {
          // The element is not there yet — keep polling.
        }
        await sleep(200);
      }
      throw new Error(`Timeout: ${label}`);
    },
  };
  return s;
}
