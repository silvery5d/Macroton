#!/usr/bin/env node

// Read-only diagnostic for the local Steam CEF endpoint exposed by Macroton.
// It intentionally accepts no arbitrary JavaScript and only inspects page
// metadata/text, so it cannot submit credentials or mutate Steam state.
const http = require("http");
const WebSocket = require("ws");

const port = Number(process.env.MACROTON_STEAM_CEF_PORT || 8080);
if (!Number.isInteger(port) || port < 1 || port > 65535) {
  console.error("MACROTON_STEAM_CEF_PORT must be a valid TCP port");
  process.exit(2);
}

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

http
  .get(`http://127.0.0.1:${port}/json`, response => {
    let body = "";
    response.setEncoding("utf8");
    response.on("data", chunk => (body += chunk));
    response.on("error", error => fail(error.message));
    response.on("end", () => {
      let targets;
      try {
        targets = JSON.parse(body);
      } catch (error) {
        fail(`invalid CEF target JSON: ${error.message}`);
      }
      const pages = targets.filter(
        target =>
          target.type === "page" &&
          typeof target.webSocketDebuggerUrl === "string" &&
          /^ws:\/\/(127\.0\.0\.1|localhost):/.test(target.webSocketDebuggerUrl),
      );
      if (!pages.length) fail("no local Steam CEF page targets");
      let remaining = pages.length;
      for (const target of pages) {
        const socket = new WebSocket(target.webSocketDebuggerUrl);
        let settled = false;
        const finish = result => {
          if (settled) return;
          settled = true;
          console.log(JSON.stringify({ target: {
            id: target.id,
            title: target.title,
            url: target.url,
          }, ...result }));
          socket.close();
          remaining -= 1;
          if (!remaining) process.exit(0);
        };
        socket.on("open", () => socket.send(JSON.stringify({
          id: 1,
          method: "Runtime.evaluate",
          params: {
            expression: `JSON.stringify({
              title: document.title,
              body: (document.body && document.body.innerText || "").slice(0, 4000),
              buttons: [...document.querySelectorAll("button,[role=button]")]
                .map(e => (e.innerText || e.textContent || "").trim())
                .filter(Boolean).slice(0, 100)
            })`,
            returnByValue: true,
          },
        })));
        socket.on("message", message => {
          try {
            const packet = JSON.parse(message.toString());
            if (packet.id !== 1) return;
            const value = packet.result?.result?.value;
            finish(typeof value === "string" ? JSON.parse(value) : { error: "no evaluation value" });
          } catch (error) {
            finish({ error: error.message });
          }
        });
        socket.on("error", error => finish({ error: error.message }));
        setTimeout(() => finish({ error: "CEF evaluation timed out" }), 5000);
      }
    });
  })
  .on("error", error => fail(`cannot reach local Steam CEF on port ${port}: ${error.message}`));
