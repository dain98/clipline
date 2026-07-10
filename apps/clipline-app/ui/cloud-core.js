// Pure Clipline Cloud request arbitration. Keep DOM- and Tauri-free for Boa tests.
var CloudCore = (() => {
  const accountKey = (cloud = {}) => [
    cloud.host_url || "",
    cloud.connected_user_id || "",
    cloud.credential_target || "",
  ].map(String).join("|");

  const createRequestGate = () => {
    let generation = 0;
    return {
      begin(key) {
        generation += 1;
        return { generation, accountKey: String(key || "") };
      },
      invalidate() {
        generation += 1;
        return generation;
      },
      isCurrent(request, key) {
        return !!request
          && request.generation === generation
          && request.accountKey === String(key || "");
      },
    };
  };

  return { accountKey, createRequestGate };
})();

globalThis.CloudCore = CloudCore;
