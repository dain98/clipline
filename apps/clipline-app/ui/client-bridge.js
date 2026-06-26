(function () {
  function requireTauri() {
    if (!window.__TAURI__) {
      throw new Error("Clipline host bridge could not find Tauri or fallback transport");
    }
    return window.__TAURI__;
  }

  const tauri = window.__TAURI__;
  const fallbackConfig = window.__CLIPLINE_FALLBACK__;

  if (tauri) {
    const appWindow = tauri.window.getCurrentWindow();
    window.cliplineHost = {
      mode: "tauri",
      invoke: (command, args) => tauri.core.invoke(command, args),
      listen: (event, handler) => tauri.event.listen(event, handler),
      convertFileSrc: (path) => tauri.core.convertFileSrc(path),
      window: {
        minimize: () => tauri.core.invoke("minimize_main_window"),
        toggleMaximize: () => appWindow.toggleMaximize(),
        close: () => appWindow.close(),
      },
    };
    return;
  }

  if (!fallbackConfig) {
    requireTauri();
  }

  window.cliplineHost = {
    mode: "fallback",
    invoke(command, args = {}) {
      return fetch(`${fallbackConfig.baseUrl}/invoke/${encodeURIComponent(command)}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(args),
      }).then(async (response) => {
        const text = await response.text();
        let payload;
        try {
          payload = parseJsonText(text, `fallback invoke ${command}`);
        } catch (error) {
          showBridgeError(error.message);
          throw error;
        }
        if (!payload || typeof payload !== "object") {
          const message = `fallback invoke ${command} returned invalid response payload`;
          showBridgeError(message);
          throw message;
        }
        if (!response.ok || payload.ok === false) {
          const message = payload.error || `command failed: ${command}`;
          showBridgeError(message);
          throw message;
        }
        if (!Object.prototype.hasOwnProperty.call(payload, "value")) {
          const message = `fallback invoke ${command} returned invalid response payload`;
          showBridgeError(message);
          throw message;
        }
        return payload.value;
      });
    },
    listen(event, handler) {
      return fallbackSubscribe(fallbackConfig, event, handler);
    },
    convertFileSrc(path) {
      return `${fallbackConfig.baseUrl}/media/${encodeURIComponent(path)}`;
    },
    window: {
      minimize: () => fallbackWindowAction(fallbackConfig, "minimize"),
      toggleMaximize: () => fallbackWindowAction(fallbackConfig, "toggle_maximize"),
      close: () => fallbackWindowAction(fallbackConfig, "close"),
    },
  };

  function fallbackWindowAction(config, action) {
    return fetch(`${config.baseUrl}/window/${action}`, { method: "POST" }).then((response) => {
      if (!response.ok) {
        const message = `window action failed: ${action} (${response.status})`;
        showBridgeError(message);
        throw message;
      }
      return null;
    });
  }

  function fallbackSubscribe(config, event, handler) {
    const source = new EventSource(`${config.baseUrl}/events?name=${encodeURIComponent(event)}`);
    source.addEventListener(event, (message) => {
      let payload;
      try {
        payload = parseJsonText(message.data, `fallback event ${event}`);
      } catch (error) {
        showBridgeError(error.message);
        return;
      }
      handler({ event, payload });
    });
    source.onerror = () => {
      showBridgeError("Clipline fallback event stream disconnected");
    };
    return Promise.resolve(() => source.close());
  }

  function showBridgeError(message) {
    const error = document.getElementById("error");
    if (error) error.textContent = message;
  }

  function parseJsonText(text, context) {
    try {
      return JSON.parse(text);
    } catch {
      throw new Error(`${context} returned malformed JSON`);
    }
  }
})();
