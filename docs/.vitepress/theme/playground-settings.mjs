const defaults = { lineWidth: 80, indentWidth: 2, roxygen: true };

function width(value, fallback) {
  if (!/^\d+$/.test(value ?? "")) return fallback;
  const number = Number(value);
  return number >= 1 && number <= 1000 ? number : fallback;
}

export function readSettings(search) {
  const params = new URLSearchParams(search);
  return {
    lineWidth: width(params.get("lw"), defaults.lineWidth),
    indentWidth: width(params.get("iw"), defaults.indentWidth),
    roxygen: params.get("roxygen") !== "0",
  };
}

export function settingsUrl(location, settings) {
  const url = new URL(location);
  url.search = new URLSearchParams({
    lw: settings.lineWidth,
    iw: settings.indentWidth,
    roxygen: settings.roxygen ? "1" : "0",
  }).toString();
  url.hash = "";
  return url;
}
