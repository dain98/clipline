// Shared DOM-free presentation policy. Classic global is the compatibility
// adapter for Boa and the remaining legacy controllers; presentation.mjs is
// the explicit ES-module surface used by the live bootstrap.
var PresentationCore = (() => {
  const MONTHS = Object.freeze([
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ]);
  const DAYS = Object.freeze([
    "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
  ]);

  const clipNameStem = (name) => String(name || "")
    .replace(/\.(mp4|mov|mkv|webm)$/i, "")
    .trim();

  const markerKindLabel = (kind, configuredLabel = "") => {
    const normalizedKind = String(kind || "Other");
    const configured = String(configuredLabel || "").trim();
    return configured || normalizedKind.replace(/([a-z])([A-Z])/g, "$1 $2");
  };

  const monthName = (month0) => MONTHS[month0] || "";
  const dayName = (day0) => DAYS[day0] || "";

  const formatClipTitle = (month0, day, hours, minutes) => {
    const h12 = hours % 12 === 0 ? 12 : hours % 12;
    const ampm = hours < 12 ? "AM" : "PM";
    return `${monthName(month0)} ${day} · ${h12}:${String(minutes).padStart(2, "0")} ${ampm}`;
  };

  const formatGalleryDay = (date) => `${dayName(date.getDay())}, ${monthName(date.getMonth())} ${date.getDate()}`;

  return Object.freeze({
    clipNameStem,
    markerKindLabel,
    monthName,
    dayName,
    formatClipTitle,
    formatGalleryDay,
  });
})();

globalThis.PresentationCore = PresentationCore;
