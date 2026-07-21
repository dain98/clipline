const PresentationCore = globalThis.PresentationCore;
if (!PresentationCore) throw new Error("presentation-core.js must load before presentation.mjs");

export { PresentationCore };
export const {
  clipNameStem,
  markerKindLabel,
  monthName,
  dayName,
  formatClipTitle,
  formatGalleryDay,
} = PresentationCore;
