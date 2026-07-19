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

  const backendOwnedCloudFields = [
    "host_url",
    "public_url",
    "connected_user_id",
    "connected_username",
    "connected_display_name",
    "credential_target",
    "uploads",
  ];

  const mergeBackendCloudSettings = (localSettings = {}, backendSettings = {}) => {
    const backendCloud = backendSettings.cloud || {};
    const cloud = { ...(localSettings.cloud || {}) };
    for (const field of backendOwnedCloudFields) {
      cloud[field] = field === "uploads"
        ? { ...(backendCloud.uploads || {}) }
        : (backendCloud[field] ?? null);
    }
    return { ...localSettings, cloud };
  };

  return { accountKey, createRequestGate, mergeBackendCloudSettings };
})();

globalThis.CloudCore = CloudCore;
