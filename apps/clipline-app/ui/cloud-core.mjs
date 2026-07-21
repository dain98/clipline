const CloudCore = globalThis.CloudCore;
if (!CloudCore) throw new Error("cloud-core.js must load before cloud-core.mjs");
export { CloudCore };
