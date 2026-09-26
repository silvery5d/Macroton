#!/usr/bin/env node
// Host-side CEF capture. This deliberately avoids launching a PE probe under
// Wine: on Apple Silicon the Wine WebSocket client can hang even while the
// same-prefix CEF endpoint is healthy.
const fs = require("fs");

const output = process.argv[2] || "/private/tmp/macroton-steam-login-qr.png";
const deadline = Date.now() + Number(process.argv[3] || 60) * 1000;
const cefPort = process.env.MACROTON_STEAM_CEF_PORT || "8080";
try {
  fs.rmSync(output, { force: true });
} catch {
  // A stale capture must never be mistaken for the current login QR.
}

async function targets() {
  try {
    const response = await fetch(`http://127.0.0.1:${cefPort}/json`);
    const value = await response.json();
    return Array.isArray(value) ? value : [];
  } catch {
    return [];
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function command(target, method, params = {}) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    const timer = setTimeout(() => {
      socket.close();
      reject(new Error("CEF WebSocket timeout"));
    }, 5000);
    socket.onerror = () => {
      clearTimeout(timer);
      reject(new Error("CEF WebSocket error"));
    };
    socket.onopen = () => socket.send(JSON.stringify({ id: 1, method, params }));
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (message.id !== 1) return;
      clearTimeout(timer);
      socket.close();
      resolve(message);
    };
  });
}

(async () => {
  while (Date.now() < deadline) {
    const target = (await targets()).find(
      (item) => item.title === "Sign in to Steam" && item.webSocketDebuggerUrl,
    );
    if (target) {
      try {
        const ready = await command(target, "Runtime.evaluate", {
          expression: "JSON.stringify({ready:[...document.images].some(i=>i.complete&&i.naturalWidth>=32&&i.naturalHeight>=32&&i.getBoundingClientRect().width>=128&&i.getBoundingClientRect().height>=128)})",
          returnByValue: true,
        });
        const value = ready.result?.result?.value;
        if (value && JSON.parse(value).ready) {
          const shot = await command(target, "Page.captureScreenshot", {
            format: "png",
            fromSurface: true,
          });
          if (typeof shot.result?.data === "string") {
            fs.writeFileSync(output, Buffer.from(shot.result.data, "base64"), { mode: 0o600 });
            console.log(output);
            process.exit(0);
          }
        }
      } catch {
        // Steam may replace the target while the login page is loading.
      }
    }
    await sleep(250);
  }
  process.exit(1);
})();
