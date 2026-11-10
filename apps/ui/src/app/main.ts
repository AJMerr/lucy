// Lucy's main window: chats on the left, the open chat on the right. In the desktop app it
// reaches the core through the app's Rust side; opened in a browser (or a phone app later),
// it uses the core's remote access: `app.html?url=ws://desktop:7077/rpc&token=…`.

import { mount } from "svelte";

import { SocketTransport, type Transport, webSocket } from "../lib/rpc/transport.ts";
import { TauriTransport } from "../lib/rpc/tauri.ts";
import App from "./App.svelte";

const CLIENT = "lucy-ui 0.7.0";

function transport(): Transport {
  if ("__TAURI_INTERNALS__" in window) {
    return new TauriTransport();
  }
  const params = new URLSearchParams(location.search);
  const url = params.get("url") ?? `ws://${location.hostname || "127.0.0.1"}:7077/rpc`;
  return new SocketTransport(webSocket(url, params.get("token") ?? ""), { client: CLIENT });
}

const target = document.getElementById("app");
if (target === null) {
  throw new Error("missing #app");
}
mount(App, { target, props: { core: transport() } });
