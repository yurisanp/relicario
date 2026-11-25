// Confere a interface sem navegador: sintaxe dos scripts e se toda chave de texto usada
// existe em português e em inglês. Roda no CI (`node crates/relicario-ui/ui/check.mjs`).
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const dir = dirname(fileURLToPath(import.meta.url));
const read = (f) => readFileSync(join(dir, f), "utf8");
const app = read("app.js"), i18n = read("i18n.js"), html = read("index.html");
for (const [f, src] of [["app.js", app], ["i18n.js", i18n]]) new vm.Script(src, { filename: f });

const ctx = { window: {}, navigator: { language: "pt-BR" }, localStorage: { getItem: () => null } };
vm.runInNewContext(i18n, ctx);
const { pt, en } = ctx.window.I18N.keys();
const used = new Set([
  ...[...app.matchAll(/\bt\("([^"]+)"/g)].map((m) => m[1]),
  ...[...html.matchAll(/data-t[pa]?="([^"]+)"/g)].map((m) => m[1]),
]);
const errors = [];
for (const k of used) if (!k.endsWith(".") && !pt.includes(k)) errors.push(`falta em pt-BR: ${k}`);
for (const k of pt) if (!en.includes(k)) errors.push(`falta em inglês: ${k}`);
for (const k of en) if (!pt.includes(k)) errors.push(`só em inglês: ${k}`);
if (errors.length) {
  console.error(errors.join("\n"));
  process.exit(1);
}
console.log(`interface ok: ${used.size} textos usados, ${pt.length} traduzidos`);
