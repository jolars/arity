import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import init, {
  check,
  format,
} from "../.vitepress/dist/playground/pkg/arity_wasm.js";
import { distRoot } from "../scripts/content.mjs";

await init({
  module_or_path: await readFile(
    join(distRoot, "playground/pkg/arity_wasm_bg.wasm"),
  ),
});

test("browser bindings format R and enforce options through JavaScript", () => {
  assert.equal(format("x=1"), "x = 1");
  assert.equal(format("x=1\n"), "x = 1\n");
  assert.equal(format(""), "");
  const source = "f=function(x,y){list(first=x,second=y,third=x+y)}";
  const formatted = format(source, 30, 4, true);
  assert.match(formatted, /\n {4}\S/);
  assert.equal(format(formatted, 30, 4, true), formatted);
  assert.throws(() => format("x=1", 0), /line-width/);
  assert.throws(() => format("x=1", 80, 1001), /indent-width/);
  assert.throws(() => format("f("), /parser diagnostic/);
  assert.equal(
    format("# arity-format skip-file\r\nx=  1\r\n"),
    "# arity-format skip-file\r\nx=  1\r\n",
  );
  const roxygen = "#' @param x   Some input.\nf=function(x){x}\n";
  assert.ok(
    format(roxygen, undefined, undefined, false).startsWith(
      "#' @param x   Some input.\n",
    ),
  );
});

test("browser diagnostics use UTF-16 positions and can be freed", () => {
  assert.deepEqual(check("x=1"), []);
  const diagnostics = check("# 😀é\n)");
  try {
    assert.equal(diagnostics.length, 1);
    const diagnostic = diagnostics[0];
    assert.equal(diagnostic.message, "unexpected ')'");
    assert.equal(diagnostic.start, 6);
    assert.equal(diagnostic.end, 7);
    assert.equal(diagnostic.line, 2);
    assert.equal(diagnostic.column, 1);
  } finally {
    diagnostics.forEach((diagnostic) => diagnostic.free());
  }
  const incomplete = check("f(");
  try {
    assert.ok(incomplete.length > 0);
    for (const diagnostic of incomplete) {
      assert.ok(diagnostic.start <= diagnostic.end);
      assert.ok(diagnostic.end <= 2);
    }
  } finally {
    incomplete.forEach((diagnostic) => diagnostic.free());
  }
});
