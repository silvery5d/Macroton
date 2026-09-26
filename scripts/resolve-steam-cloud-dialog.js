#!/usr/bin/env node
// Inspect, or explicitly resolve, a Steam Cloud launch dialog through the
// localhost-only CEF endpoint. The default is read-only. `--apply` is required
// before clicking a button, and only exact continuation labels are accepted.
import { readFileSync } from "node:fs";

const options = process.argv.slice(2);
const apply = options.includes("--apply");
let webhelperLog;
for (let index = 0; index < options.length; index += 1) {
  if (options[index] === "--webhelper-log") {
    webhelperLog = options[index + 1];
    index += 1;
  }
}
const unknown = options.filter(
  (option, index) =>
    option !== "--apply" &&
    option !== "--webhelper-log" &&
    !(options[index - 1] === "--webhelper-log"),
);
if (unknown.length) {
  console.error(`unknown option(s): ${unknown.join(", ")}`);
  console.error(
    "usage: node resolve-steam-cloud-dialog.js [--webhelper-log PATH] [--apply]",
  );
  process.exit(2);
}
if (options.includes("--webhelper-log") && (!webhelperLog || webhelperLog.startsWith("--"))) {
  console.error("--webhelper-log requires a path");
  process.exit(2);
}
if (apply && process.env.MACROTON_ALLOW_SESSION_DEBUGGING !== "1") {
  console.error(
    "refusing to click a Steam session without MACROTON_ALLOW_SESSION_DEBUGGING=1",
  );
  process.exit(2);
}
let port = process.env.MACROTON_STEAM_CEF_PORT || "8080";
if (webhelperLog) {
  let log;
  try {
    log = readFileSync(webhelperLog, "utf8");
  } catch (error) {
    console.error(`cannot read Steam webhelper log ${webhelperLog}: ${error.message}`);
    process.exit(1);
  }
  const matches = [...log.matchAll(
    /--remote-debugging-address=127\.0\.0\.1[^\n]*?--remote-debugging-port=(\d+)/g,
  )];
  if (matches.length) port = matches[matches.length - 1][1];
}
if (!/^[0-9]+$/.test(port) || Number(port) < 1 || Number(port) > 65535) {
  console.error("MACROTON_STEAM_CEF_PORT must be a numeric TCP port from 1 to 65535");
  process.exit(2);
}
const endpoint = `http://127.0.0.1:${port}/json`;

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

let targets;
try {
  targets = await (await fetch(endpoint)).json();
} catch (error) {
  console.error(`cannot connect to local Steam CEF endpoint ${endpoint}: ${error.message}`);
  process.exit(1);
}
const pages = targets.filter((target) => target.type === "page" && target.webSocketDebuggerUrl);
if (!pages.length) {
  console.error(`no Steam CEF pages found at ${endpoint}`);
  process.exit(1);
}

const inspect = `(() => {
  const elements = [...document.querySelectorAll('button,[role=button]')];
  return JSON.stringify({
    title: document.title,
    text: document.body?.innerText?.slice(0, 2000) || '',
    buttons: elements.map((e) => (e.innerText || e.textContent || '').trim()).filter(Boolean)
  });
})()`;
const allowed = /^(play anyway|continue|launch anyway|start anyway)$/i;
let found = false;
for (const target of pages) {
  try {
    const result = await command(target, "Runtime.evaluate", { expression: inspect, returnByValue: true });
    const raw = result.result?.result?.value;
    if (!raw) continue;
    const info = JSON.parse(raw);
    const relevant = info.buttons.filter((button) => allowed.test(button));
    console.log(JSON.stringify({ target: info.title, relevant_buttons: relevant, read_only: !apply }));
    if (!apply || !relevant.length) continue;
    const click = `(() => {
      const allowed = /^(play anyway|continue|launch anyway|start anyway)$/i;
      const button = [...document.querySelectorAll('button,[role=button]')]
        .find((e) => allowed.test((e.innerText || e.textContent || '').trim()));
      if (!button) return JSON.stringify({ clicked: false });
      const label = (button.innerText || button.textContent || '').trim();
      button.click();
      return JSON.stringify({ clicked: true, label });
    })()`;
    const clicked = await command(target, "Runtime.evaluate", { expression: click, returnByValue: true });
    console.log(clicked.result?.result?.value || JSON.stringify({ clicked: false }));
    found = true;
    break;
  } catch (error) {
    console.error(`${infoLabel(target)}: ${error.message}`);
  }
}
if (apply && !found) {
  console.error("no approved Steam Cloud continuation button was found");
  process.exit(1);
}

function infoLabel(target) {
  return target.title || target.url || "Steam CEF page";
}
