const PlayerCore = globalThis.PlayerCore;
if (!PlayerCore) throw new Error("player-core.js must load before player-core.mjs");
export { PlayerCore };
