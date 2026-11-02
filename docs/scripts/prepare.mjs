import { copyFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { docsRoot, projectRoot } from "./content.mjs";

const publicRoot = join(docsRoot, "public");
mkdirSync(publicRoot, { recursive: true });
for (const [source, target] of [
  ["scripts/arity-installer.sh", "install"],
  ["scripts/arity-installer.ps1", "install.ps1"],
  ["arity.schema.json", "arity.schema.json"],
])
  copyFileSync(join(projectRoot, source), join(publicRoot, target));
