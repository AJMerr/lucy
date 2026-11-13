// Lucy's main window: chats on the left, the open chat on the right. In the desktop app it
// reaches the core through the app's Rust side; opened in a browser (or a phone app later),
// it uses the core's remote access: `app.html?url=ws://desktop:7077/rpc&token=…`.

import { mount } from "svelte";

import { SocketTransport, type Transport, webSocket } from "../lib/rpc/transport.ts";
import { TauriTransport } from "../lib/rpc/tauri.ts";
import { VERSION } from "../lib/version.ts";
import App from "./App.svelte";

const CLIENT = `lucy-ui ${VERSION}`;

function transport(): Transport {
  if ("__TAURI_INTERNALS__" in window) {
    return new TauriTransport();
  }
  // The token comes after `#` (never sent to a server, and taken out of the address bar
  // straight away so it doesn't stay in history); the core's address only from this page's own
  // host unless you confirm another.
  const params = new URLSearchParams(location.hash.slice(1) || location.search);
  const token = params.get("token") ?? "";
  if (location.hash !== "" || params.has("token")) {
    history.replaceState(null, "", location.pathname);
  }
  const own = `wss://${location.hostname || "127.0.0.1"}:7077/rpc`;
  const asked = params.get("url");
  const elsewhere = (address: string) => {
    try {
      return new URL(address).hostname !== location.hostname;
    } catch {
      return true;
    }
  };
  const url =
    asked !== null && elsewhere(asked)
      ? confirm(`Connect to Lucy's core at ${asked}? Only do this if you set it up.`)
        ? asked
        : own
      : (asked ?? own);
  return new SocketTransport(webSocket(url, token), { client: CLIENT });
}

const target = document.getElementById("app");
if (target === null) {
  throw new Error("missing #app");
}
mount(App, { target, props: { core: transport() } });
