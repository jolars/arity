import assert from "node:assert/strict";
import test from "node:test";
import {
  readSettings,
  settingsUrl,
} from "../.vitepress/theme/playground-settings.mjs";

test("missing or invalid URL settings use the formatter defaults", () => {
  const defaults = { lineWidth: 80, indentWidth: 2, roxygen: true };
  assert.deepEqual(readSettings(""), defaults);
  assert.deepEqual(
    readSettings("?lw=0&iw=1001&roxygen=invalid&code=x%3D1"),
    defaults,
  );
  for (const width of ["", "-1", "2.5", "NaN", "Infinity", "0x20"]) {
    assert.equal(readSettings(`?lw=${width}`).lineWidth, 80);
  }
  assert.deepEqual(readSettings("?lw=40&iw=4&roxygen=0"), {
    lineWidth: 40,
    indentWidth: 4,
    roxygen: false,
  });
});

test("share links contain only settings and round-trip them", () => {
  const settings = { lineWidth: 100, indentWidth: 4, roxygen: false };
  const url = settingsUrl(
    "https://arity.cc/playground.html?code=private#source",
    settings,
  );
  assert.equal(
    url.href,
    "https://arity.cc/playground.html?lw=100&iw=4&roxygen=0",
  );
  assert.deepEqual(readSettings(url.search), settings);
});
